//! Run against the real, windowless native MCP host:
//! PM_MCP_BENCH_BIN=/absolute/path/to/powermove-native \
//!   cargo test -p pm-mcp --test mcp_speed -- --ignored --nocapture
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdout, Command, Stdio},
    time::Instant,
};

struct Server {
    child: Child,
    reader: BufReader<ChildStdout>,
    root: PathBuf,
    id: u64,
    bytes: usize,
}

impl Server {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(pm_kernel::new_id("mcp-speed"));
        fs::create_dir(&root).unwrap();
        let mut child = Command::new(std::env::var_os("PM_MCP_BENCH_BIN").unwrap())
            .arg("--powermove-mcp")
            .arg("--profile")
            .arg(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            reader,
            root,
            id: 0,
            bytes: 0,
        };
        server.request("initialize", json!({"protocolVersion":"2025-11-25"}));
        server
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let request = json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        assert_ne!(self.reader.read_line(&mut line).unwrap(), 0, "{method}");
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], self.id);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        let result = self.request("tools/call", json!({"name":name,"arguments":args}));
        assert_eq!(result["isError"], false, "{name}: {result}");
        self.bytes += result.to_string().len();
        result
    }

    fn text(&mut self, name: &str, args: Value) -> Value {
        let result = self.tool(name, args);
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn job(&mut self, name: &str, duration: u64, run: impl FnOnce(&mut Self) -> Value) -> Value {
        let started = Instant::now();
        let (id, bytes) = (self.id, self.bytes);
        self.text(
            "create_project",
            json!({"name":name,"width":1920,"height":1080,"fps":30,"duration":duration}),
        );
        self.text("start_session", json!({"prompt":name}));
        let stats = run(self);
        self.text("finish_session", json!({"commit":true}));
        json!({
            "job":name,"calls":self.id-id,"wall_ms":started.elapsed().as_millis() as u64,
            "result_bytes":self.bytes-bytes,"details":stats
        })
    }

    fn edit(&mut self, commands: Vec<Value>) -> Value {
        self.text("apply_commands", json!({"commands":commands}))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.stdin.take();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn keys(target: &str, path: &str, prefix: &str, end: f64, first: i32, last: i32) -> Value {
    json!({"type":"replace_keyframes","target":target,"path":path,"keyframes":[
        {"id":format!("{prefix}-start"),"time":0,"value":first},
        {"id":format!("{prefix}-end"),"time":end,"value":last}
    ]})
}

#[test]
#[ignore = "requires the native executable and headless render/export backend"]
fn standard_jobs() {
    let mut server = Server::new();
    let title = server.job("title_animation", 10, |server| {
        server.edit(vec![
            json!({"type":"add_layer","id":"title","layerType":"text","duration":10,
                   "content":{"text":"Powermove","size":96}}),
            keys("title", "opacity", "opacity", 2., 20, 100),
            keys("title", "position.x", "position", 2., 700, 960),
            keys("title", "scale.x", "scale", 2., 80, 100),
            json!({"type":"set_easing","keyframes":["opacity-start","position-start","scale-start"],
                   "curve":[0.125,0.25,0.75,0.875]}),
        ]);
        let frames = server.tool("render_frames", json!({"times":[0,1,9.5],"width":480}));
        let images = frames["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["type"] == "image")
            .count();
        assert_eq!(images, 3);
        json!({"check_frames":images})
    });
    let lower = server.job("lower_third", 10, |server| {
        let edit = server.edit(vec![
            json!({"type":"add_layer","id":"plate","layerType":"shape","duration":10,
                   "content":{"shape":"rect","w":600,"h":140,"color":"#14304c"}}),
            json!({"type":"add_layer","id":"label","layerType":"text","duration":10,
                   "content":{"text":"Ada North","size":55}}),
            json!({"type":"set_property","target":"plate","path":"position.y","value":884,"mode":"static"}),
            json!({"type":"set_property","target":"label","path":"position.y","value":884,"mode":"static"}),
            keys("plate", "opacity", "plate", 1., 40, 100),
            keys("label", "opacity", "label", 1., 10, 100),
            json!({"type":"group_layers","targets":["plate","label"],"name":"Lower third"}),
        ]);
        let group = edit["results"][6]["data"]["id"].as_str().unwrap();
        server.edit(vec![
            json!({"type":"set_property","target":group,"path":"anchor.x","value":50,"mode":"static"}),
        ]);
        json!({"grouped_layers":2})
    });
    let export = server.job("h264_1080p_2s", 2, |server| {
        server.edit(vec![
            json!({"type":"add_layer","id":"plate","layerType":"shape","duration":2,
                   "content":{"shape":"rect","w":600,"h":140,"color":"#14304c"}}),
        ]);
        let start = Instant::now();
        let delivery = server.text(
            "export_video",
            json!({"format":"mp4","codec":"h264","range":"full","audio":false}),
        );
        let submit_ms = start.elapsed().as_millis() as u64;
        let output = PathBuf::from(delivery["path"].as_str().expect("export output path"));
        assert!(output.starts_with(&server.root));
        assert!(fs::metadata(output).unwrap().len() > 0);
        json!({"export_reply_ms":submit_ms})
    });
    println!("MCP_SPEED_JSON {}", json!({"jobs":[title,lower,export]}));
}
