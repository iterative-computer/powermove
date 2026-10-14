//! Channel dispatch: the Rust stand-in for Electron's `ipcMain`.
//!
//! Every IPC channel from `apps/desktop/src/shared/ipc.ts` that the host
//! answers is registered once on a [`Router`]. `invoke` and `sync` messages
//! reply with the handler's value; `send` messages ignore it.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::host::Host;
use crate::wire::{self, Payload};

/// One connected editor window (a WebView tab speaking the wire protocol).
pub struct Client {
    pub id: u64,
    outbox: mpsc::UnboundedSender<Vec<u8>>,
    asks: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_ask: AtomicU64,
    closed: AtomicBool,
    on_close: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    /// The project this window claimed (`window:claim-project`).
    pub project: Mutex<Option<String>>,
}

impl Client {
    pub fn new(id: u64, outbox: mpsc::UnboundedSender<Vec<u8>>, project: Option<String>) -> Self {
        Self {
            id,
            outbox,
            asks: Mutex::default(),
            next_ask: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            on_close: Mutex::default(),
            project: Mutex::new(project),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Push an event (`webContents.send`). `args` is the event's argument list.
    pub fn send(&self, channel: &str, args: Payload) {
        self.push(wire::event(channel, args));
    }

    /// Convenience for a single JSON argument.
    pub fn emit(&self, channel: &str, arg: Value) {
        self.send(channel, Payload::json(Value::Array(vec![arg])));
    }

    /// Ask the window to decide something and wait for its answer.
    #[allow(dead_code)]
    pub async fn ask(&self, channel: &str, args: Vec<Value>) -> Result<Value> {
        let id = self.next_ask.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.asks.lock().insert(id, sender);
        self.push(wire::ask(id, channel, Payload::json(Value::Array(args))));
        receiver.await.map_err(|_| anyhow!("Powermove window closed."))
    }

    /// Run `callback` once when this window disconnects.
    pub fn on_close(&self, callback: impl FnOnce() + Send + 'static) {
        if self.is_closed() {
            callback();
        } else {
            self.on_close.lock().push(Box::new(callback));
        }
    }

    pub(crate) fn answer(&self, id: u64, value: Value) {
        if let Some(sender) = self.asks.lock().remove(&id) {
            let _ = sender.send(value);
        }
    }

    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.asks.lock().clear();
        for callback in std::mem::take(&mut *self.on_close.lock()) {
            callback();
        }
    }

    pub(crate) fn push(&self, payload: Payload) {
        if !self.is_closed() {
            let _ = self.outbox.send(wire::encode(&payload));
        }
    }
}

/// One request as a handler sees it.
pub struct Call {
    pub host: Arc<Host>,
    pub client: Arc<Client>,
    pub args: Vec<Value>,
    pub blobs: Arc<Vec<Bytes>>,
}

impl Call {
    /// The `index`th argument, or `null` when absent (JS `undefined`).
    pub fn arg(&self, index: usize) -> &Value {
        self.args.get(index).unwrap_or(&Value::Null)
    }

    /// Deserialize the first argument into a typed request.
    pub fn parse<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(self.arg(0).clone()).context("invalid request")
    }

    /// The bytes behind a `{"$bin": n}` placeholder.
    pub fn bytes(&self, value: &Value) -> Result<Bytes> {
        wire::blob(value, &self.blobs).cloned().context("expected binary data")
    }
}

pub type Reply = Result<Payload>;
type Handler = Arc<dyn Fn(Call) -> BoxFuture<'static, Reply> + Send + Sync>;

#[derive(Default)]
pub struct Router {
    handlers: HashMap<String, Handler>,
}

impl Router {
    /// Register the handler for `channel`. Registering twice is a bug.
    pub fn on<F, Fut>(&mut self, channel: &str, handler: F)
    where
        F: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Reply> + Send + 'static,
    {
        let previous = self.handlers.insert(channel.to_owned(), Arc::new(move |call| Box::pin(handler(call))));
        assert!(previous.is_none(), "second handler registered for '{channel}'");
    }

    /// Register a handler that returns plain JSON.
    pub fn json<F, Fut>(&mut self, channel: &str, handler: F)
    where
        F: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.on(channel, move |call| {
            let handler = handler.clone();
            async move { handler(call).await.map(Payload::json) }
        });
    }

    /// Like [`Router::on`], with a clone of `state` handed to each call.
    pub fn on_with<S, F, Fut>(&mut self, state: &S, channel: &str, handler: F)
    where
        S: Clone + Send + Sync + 'static,
        F: Fn(S, Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Reply> + Send + 'static,
    {
        let state = state.clone();
        self.on(channel, move |call| handler(state.clone(), call));
    }

    /// Like [`Router::json`], with a clone of `state` handed to each call.
    pub fn json_with<S, F, Fut>(&mut self, state: &S, channel: &str, handler: F)
    where
        S: Clone + Send + Sync + 'static,
        F: Fn(S, Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let state = state.clone();
        self.json(channel, move |call| handler(state.clone(), call));
    }

    /// Answer `channel` with a fixed value (features this host does not offer yet).
    pub fn constant(&mut self, channel: &str, value: Value) {
        self.json(channel, move |_| {
            let value = value.clone();
            async move { Ok(value) }
        });
    }

    pub fn has(&self, channel: &str) -> bool {
        self.handlers.contains_key(channel)
    }

    pub(crate) fn get(&self, channel: &str) -> Option<Handler> {
        self.handlers.get(channel).cloned()
    }
}
