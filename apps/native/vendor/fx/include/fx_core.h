/*
 * fx_core.h: the C ABI of Powermove's libfx core (patch P-CAPI), ABI 2.
 *
 * Powermove notice: new file, not in upstream fx. Copyright the Powermove
 * authors; licensed under Apache-2.0 like the rest of this vendored tree.
 * Implemented by src/capi_core_main.zig. See POWERMOVE.md.
 *
 * One runtime is one agent session on its own core thread. The host (Rust,
 * the `pm-aero` worker) drives it with the calls below and drains what the
 * core produces: typed events, model HTTP requests, tool calls, journal
 * appends and hook questions.
 *
 * ========================================================================
 * RULES THAT HOLD FOR EVERY FUNCTION
 * ========================================================================
 *
 * 1. Threads.
 *    No function may be called on the process's main thread, except
 *    fx_core_abi_version, which any thread may call. Functions marked WORKER
 *    are called by one thread at a time per runtime: the host serialises them
 *    (the thread may change between calls if each call happens-before the
 *    next). Functions marked ANY may be called from any thread, concurrently
 *    with each other and with WORKER calls. fx_host callbacks run on
 *    core-owned threads (the runtime's thread or one of its prompt or tool
 *    workers), never on a host thread, and never while a call into this API
 *    is on that thread's stack; a callback must not call this API.
 *
 * 2. Liveness. No function waits for the core to do work, except
 *    fx_runtime_destroy (below). A function may wait for a lock the core
 *    holds, but only while the holder is running: core threads hold the locks
 *    host calls take only for bounded work and never wait while holding one.
 *    The wait ends as soon as the runtime fails (the call then returns
 *    FX_E_FAILED), and after FX_BUSY_BOUND_MS at the latest (the call then
 *    returns FX_BUSY having changed nothing). A core panic never leaves a
 *    host call waiting: see "Panics" below.
 *
 * 3. Handles. fx_runtime is an opaque generation-tagged 64-bit value, never a
 *    pointer; 0 is never valid. A handle names one runtime for ever: once
 *    fx_runtime_destroy has started on it, or for a value that was never
 *    issued, every call returns FX_E_CLOSED (or the documented sentinel)
 *    without side effects, even after its slot holds a new runtime. A slot
 *    whose generations run out is never reused. Ids inside a runtime
 *    (requests, turns, tool calls, fetches, journal appends, hook questions)
 *    are 64-bit, start at 1, increase, and never repeat within the runtime.
 *
 * 4. Statuses and precedence. Every fx_status function returns FX_OK or one
 *    of the codes below. Non-negative codes are outcomes, not errors. When
 *    several apply, the first of this list wins:
 *      FX_E_UNAVAILABLE  the stub build (every function);
 *      FX_E_INVALID      a NULL, misaligned or too-small pointer or size
 *                        argument (checked before the handle);
 *      FX_E_CLOSED       the handle is destroyed or was never issued;
 *      FX_E_FAILED       the runtime is not RUNNING (exited, failed or
 *                        panicked; fx_runtime_info_get says which), for every
 *                        function that needs a live core;
 *      FX_BUSY           a lock stayed contended past the bound;
 *      then the function's own outcomes and errors, as each lists them.
 *    Functions that return a value instead of a status document their
 *    sentinel.
 *
 * 5. Outputs. Every out pointer must be non-NULL and aligned for its type,
 *    or the call returns FX_E_INVALID and writes nothing. Otherwise every
 *    out scalar (handle, request, turn, count) is written on every return:
 *    the value on FX_OK, 0 on any other result. An out struct is written in
 *    full (up to its struct_size) on FX_OK and zeroed (up to its struct_size)
 *    on any other result.
 *
 * 6. Sizes and growth (ABI evolution). Structs the host passes in begin with
 *    uint32_t struct_size, set to sizeof() as the host compiled them. The
 *    core accepts a struct_size of at least the FX_*_SIZE_V2 below; it reads
 *    only the fields that fit entirely within struct_size, and bytes beyond
 *    the fields it knows must be zero (else FX_E_UNSUPPORTED). Out structs
 *    (fx_runtime_info, fx_fetch_request) carry struct_size set by the host
 *    before the call; the core writes only that many bytes. fx_events_drain
 *    takes the element stride of the host's fx_event array. Array elements
 *    the host passes in (fx_tool_desc, fx_image, fx_named_bytes) and those
 *    the core hands out (fx_header, fx_str) are frozen: they never grow; a
 *    richer element is a new struct with a new field or function. Callback
 *    signatures and fx_host field offsets are frozen; new callbacks are
 *    appended. New event kinds, statuses and flags are additive: hosts ignore
 *    kinds and flags they do not know. FX_CORE_ABI_VERSION changes only on
 *    an incompatible change.
 *
 * 7. Strings and bytes. fx_str is UTF-8 with an explicit length and no NUL
 *    terminator; fx_bytes is raw bytes. A NULL pointer is allowed only with
 *    length 0. Everything the host hands into a function is either copied
 *    before the function returns or read only during it; the core never
 *    keeps a host pointer. fx_tool_result copies its text exactly once.
 *
 * 8. Borrows the core hands out. A pointer the core hands out stays valid
 *    for the window each function or callback states, and on a runtime that
 *    failed or panicked for as long as the process lives (its memory is
 *    abandoned, never freed). The host never frees one. Reading a borrowed
 *    pointer on another thread is allowed if the host orders that read
 *    before the call that ends the window (the call happens-after the read).
 *
 * 9. Leases. A tool call or hook question hands the host borrowed input that
 *    stays valid, from any thread, until the host answers it: tool calls with
 *    fx_tool_result, hook questions with fx_hook_answer. Answering ends the
 *    lease: the host must not read that input once it has made the call. The
 *    core waits for every lease to be answered, even a cancelled one; only
 *    fx_runtime_destroy ends leases without answers (rule 10). Each lease is
 *    answered exactly once: a second answer is FX_STALE.
 *
 * 10. Teardown. Before calling fx_runtime_destroy the host must: unregister
 *    the ready fd from every poller; stop every thread that reads a borrowed
 *    pointer of this runtime (lease inputs, events, the fetch request, the
 *    runtime info message); and not call it from a callback. host_ctx and
 *    the callback code must stay valid until fx_runtime_destroy has
 *    returned. No callback runs after it returns, on a healthy or a failed
 *    runtime.
 *
 * 11. Requests and turns: acceptance and completion. A function that starts
 *    work in the core (fx_session_open, fx_session_stage_images,
 *    fx_turn_steer, fx_turn_withdraw, fx_checkpoint) returns FX_OK when the
 *    work is accepted, with a request id; exactly one terminal event carries
 *    that id later: FX_EVENT_SESSION_OPENED, FX_EVENT_IMAGES_NEEDED,
 *    FX_EVENT_REQUEST_DONE or FX_EVENT_CHECKPOINT, as each function says.
 *    fx_turn_prompt and fx_turn_resume return FX_OK when the turn has
 *    started; its FX_EVENT_TURN_START is queued before they return, and
 *    exactly one FX_EVENT_TURN_END with its id follows. A function that does
 *    not return FX_OK started nothing. If the runtime stops (exits, fails or
 *    panics) first, FX_EVENT_EXITED is the terminal event of every request
 *    and turn still open.
 *
 * 12. Panics (P-PANIC). The core is built ReleaseSafe; a safety check that
 *    fails is a panic, and a panic never aborts the process. A panic on a
 *    core-owned thread records its message, marks the runtime PANICKED
 *    without taking a lock, wakes the ready fd and parks that thread for
 *    good; the runtime's other threads park at their next boundary with the
 *    host (a callback, an event, a host answer or a model read), so no
 *    callback starts after the failure is published. A panic inside a host
 *    call (Zig code running on the host's thread) marks the runtime
 *    PANICKED the same way and returns FX_E_FAILED from that call: the host
 *    thread is never parked. Other runtimes are unaffected. A panicked
 *    runtime is never joined or freed; fx_runtime_destroy retires its handle.
 *
 * 13. Credentials (the fetch edge). No credential ever enters the core, and
 *    the core hands out no credential-carrying header: fx_fetch_take strips
 *    authorization, proxy-authorization, cookie, x-api-key, api-key and
 *    x-goog-api-key whatever the core built (the Gateway path builds
 *    "Bearer host-managed", a placeholder). The host adds credentials only to
 *    a request whose destination origin (scheme, host and port of url) is the
 *    canonical origin of the provider fx_fetch_request.provider names, for
 *    both model and catalog requests, and never to any other destination
 *    (loopback fakes and endpoint_url overrides included).
 */
#ifndef FX_CORE_H
#define FX_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Bumped only on an incompatible change (rule 6). ABI 1 was milestone 1. */
#define FX_CORE_ABI_VERSION 2u

/* How long a host call waits for a contended lock at most (rule 2). */
#define FX_BUSY_BOUND_MS 250

typedef int32_t fx_status;
#define FX_OK              0  /* applied */
#define FX_EMPTY           1  /* nothing waiting (fx_fetch_take) */
#define FX_STALE           2  /* the id or turn is no longer current; nothing changed */
#define FX_BACKPRESSURE    3  /* a bounded queue is full; nothing changed; the ready fd wakes when it has room */
#define FX_BUSY            4  /* a lock stayed contended past FX_BUSY_BOUND_MS; nothing changed; retry */
#define FX_E_INVALID      -1  /* bad argument: NULL, misaligned, malformed, an id never issued */
#define FX_E_CLOSED       -2  /* the handle was destroyed or never issued */
#define FX_E_FAILED       -3  /* the runtime is not running; see fx_runtime_info_get */
#define FX_E_LIMIT        -4  /* the process-wide limit of 64 runtimes is reached */
#define FX_E_OOM          -5  /* the core could not allocate */
#define FX_E_THREAD       -6  /* the core could not start a thread */
#define FX_E_IO           -7  /* the readiness socket could not be made */
#define FX_E_STATE        -8  /* not valid now (prompt before open, prompt while a turn runs) */
#define FX_E_UNAVAILABLE  -9  /* this build has no core (built without Zig) */
#define FX_E_UNSUPPORTED -10  /* this build does not implement it yet, or unknown non-zero struct bytes */
#define FX_E_TOO_LARGE   -11  /* the input can never fit, however long the host waits */

typedef uint64_t fx_runtime;

typedef struct fx_str {
  const char *ptr;
  size_t len;
} fx_str; /* frozen */

typedef struct fx_bytes {
  const uint8_t *ptr;
  size_t len;
} fx_bytes; /* frozen */

/* ------------------------------------------------------------------------ */
/* Version and availability                                                 */
/* ------------------------------------------------------------------------ */

/* ANY thread, the main thread included. FX_CORE_ABI_VERSION of the linked
 * core, or 0 for the stub built without Zig, every other function of which
 * returns FX_E_UNAVAILABLE (or the function's sentinel). */
uint32_t fx_core_abi_version(void);

/* ------------------------------------------------------------------------ */
/* Limits (every one enforced; FX_E_TOO_LARGE or FX_E_INVALID beyond them)  */
/* ------------------------------------------------------------------------ */

#define FX_MAX_RUNTIMES           64u
#define FX_MAX_TOOLS             256u
#define FX_MAX_TOOL_NAME          64u        /* [A-Za-z0-9_-]{1,64} */
#define FX_MAX_TOOL_DESCRIPTION  (64u << 10)
#define FX_MAX_TOOL_SCHEMA      (256u << 10) /* a JSON object */
#define FX_MAX_INSTRUCTIONS       (1u << 20)
#define FX_MAX_SESSION_ID         255u       /* UTF-8 */
#define FX_MAX_MESSAGE_ID         128u       /* [A-Za-z0-9._:-]{1,128} */
#define FX_MAX_PROMPT_TEXT        (4u << 20) /* and its encoded request must fit FX_MAX_INPUT_QUEUE */
#define FX_MAX_STEER_TEXT        (64u << 10)
#define FX_MAX_INPUT_QUEUE        (8u << 20) /* requests the core has not read yet */
#define FX_MAX_PROMPT_IMAGES       8u
#define FX_MAX_STAGE_BATCH         8u        /* images per fx_session_stage_images */
#define FX_MAX_STAGE_BATCH_BYTES  (8u << 20)
#define FX_MAX_IMAGE_BYTES        (8u << 20)
#define FX_MAX_EVENT_BYTES       (16u << 20) /* queued event text before the core's writer waits */
#define FX_MAX_EVENTS            65536u      /* queued events before the core's writer waits */
#define FX_MAX_FETCH_CHUNK        (8u << 20) /* bytes per fx_fetch_push */
#define FX_MAX_FETCH_QUEUE        (8u << 20) /* response bytes the core has not read */
#define FX_MAX_TOOL_RESULT_LIMIT (16u << 20) /* the largest max_tool_result_bytes */
#define FX_MAX_CONCURRENT_TOOLS   16u

/* ------------------------------------------------------------------------ */
/* Host callbacks                                                           */
/* ------------------------------------------------------------------------ */

#define FX_TOOL_IDEMPOTENT (1u << 0) /* may run beside other idempotent calls */
#define FX_TOOL_WRITES     (1u << 1) /* changes state: runs alone, never re-run after a crash */

/* A host tool call (fx_host.tool_call). The struct and its strings are
 * borrowed for the lease (rule 9): until the host calls fx_tool_result for
 * `call`, or fx_runtime_destroy. input_json is the core's own copy of the
 * model's input, handed out without another copy. */
typedef struct fx_tool_call {
  uint32_t struct_size;  /* sizeof(fx_tool_call) as the core compiled it */
  uint32_t flags;        /* the tool's FX_TOOL_* */
  uint64_t call;         /* what fx_tool_result answers */
  uint64_t turn;         /* the turn that made it */
  fx_str name;
  fx_str input_json;
  fx_str model_call_id;  /* the id FX_EVENT_TOOL_START and _END carry */
} fx_tool_call;
#define FX_TOOL_CALL_SIZE_V2 72u

#define FX_APPEND_BARRIER (1u << 0) /* the core waits for this append's ack before it goes on */

/* A journal append (fx_host.journal_append), borrowed for the callback: the
 * host copies events_json. Appends arrive in order, ids 1, 2, 3, ... with no
 * gap; each holds the libfx journal events first_seq..last_seq (a JSON array,
 * seq order, consecutive with the previous append). */
typedef struct fx_journal_append {
  uint32_t struct_size;
  uint32_t flags;        /* FX_APPEND_* */
  uint64_t append;
  uint64_t turn;         /* the running turn, or 0 */
  uint64_t first_seq;
  uint64_t last_seq;
  fx_str events_json;
} fx_journal_append;
#define FX_JOURNAL_APPEND_SIZE_V2 56u

typedef uint32_t fx_hook_kind;
#define FX_HOOK_PRE_TOOL 1u /* before a host tool runs (fx's PreToolUse, P-PRETOOL) */
#define FX_HOOK_STOP     2u /* before a turn ends (fx's Stop, the completion gate, P-STOP) */

/* A hook question (fx_host.hook). Borrowed for the lease (rule 9): until the
 * host calls fx_hook_answer for `question` (returning FX_HOOK_ALLOW from the
 * callback answers it at once), or fx_runtime_destroy. */
typedef struct fx_hook_question {
  uint32_t struct_size;
  fx_hook_kind kind;
  uint64_t question;
  uint64_t turn;
  uint64_t call;          /* PRE_TOOL: the tool call it is about (not yet made) */
  fx_str name;            /* PRE_TOOL: tool name */
  fx_str input_json;      /* PRE_TOOL: the input the tool would get */
  fx_str final_text;      /* STOP: the reply the model ended with */
  uint32_t continuations; /* STOP: times this turn was already continued */
  uint32_t reserved;
} fx_hook_question;
#define FX_HOOK_QUESTION_SIZE_V2 88u

typedef int32_t fx_hook_verdict;
#define FX_HOOK_ALLOW    0  /* answered now: run the tool / end the turn */
#define FX_HOOK_PENDING  1  /* the host answers later with fx_hook_answer */

/*
 * The host's callbacks. Each runs on a core-owned thread and must return
 * quickly: it records or enqueues and never blocks, waits on the host, or
 * calls this API. host_ctx is the value given to fx_runtime_create; the core
 * never dereferences it. A NULL callback means the host does not take part:
 * no tool is called (the model reads "the host has no tools"), the session is
 * not journaled, and every hook is allowed.
 *
 * Ordering. For one call: tool_call returns before tool_cancel starts;
 * tool_cancel runs at most once, on the same thread as tool_call, and only
 * while the call is unanswered. Up to max_concurrent_tools calls are
 * unanswered at once; a call to a FX_TOOL_WRITES tool is made only when no
 * other call is unanswered, and no call is made while it is. Callbacks for
 * one runtime may run concurrently on its different threads.
 */
typedef struct fx_host {
  uint32_t struct_size; /* the core reads only callbacks that fit entirely */
  uint32_t reserved;

  void (*tool_call)(void *host_ctx, const fx_tool_call *call);
  /* The turn that made `call` was cancelled, or steered past it. The host
   * stops the work if it can and must still answer (rule 9); the core drops
   * that result. */
  void (*tool_cancel)(void *host_ctx, uint64_t call, uint64_t turn);
  void (*journal_append)(void *host_ctx, const fx_journal_append *append);
  /* NULL means every hook is allowed. A turn's stop hook is asked at most
   * max_stop_continuations + 1 times; past that the core ends the turn
   * without asking. */
  fx_hook_verdict (*hook)(void *host_ctx, const fx_hook_question *question);
} fx_host;
#define FX_HOST_SIZE_V2 40u

/* ------------------------------------------------------------------------ */
/* Runtimes                                                                 */
/* ------------------------------------------------------------------------ */

typedef struct fx_tool_desc {
  fx_str name;              /* FX_MAX_TOOL_NAME, unique in the runtime */
  fx_str description;       /* what the model reads */
  fx_str input_schema_json; /* a JSON Schema object */
  uint32_t flags;           /* FX_TOOL_* */
  uint32_t reserved;        /* zero */
} fx_tool_desc; /* frozen */

typedef struct fx_config {
  uint32_t struct_size;     /* >= FX_CONFIG_SIZE_V2 */
  uint32_t abi_version;     /* FX_CORE_ABI_VERSION, or the create fails with FX_E_UNSUPPORTED */

  /* Model selection. provider: "gateway" (empty means gateway), "codex",
   * "grok", or the name of a provider configured_providers_json defines.
   * model and effort may be empty (the core's defaults). */
  fx_str provider;
  fx_str model;
  fx_str effort;
  fx_str configured_providers_json; /* settings-style providers object, no credentials; empty if none */
  /* Gateway only (FX_E_INVALID with another provider): empty for the
   * Gateway's canonical URL, else http on 127.0.0.1, localhost or [::1] with
   * a port (local fakes and tests). Model catalog GETs still name the
   * provider's canonical URL; the host decides what reaches the network and
   * may answer any request itself (a 404 makes the core use its built-in
   * model capabilities). */
  fx_str endpoint_url;

  fx_str instructions;      /* the system instructions the model sees */
  /* Identity paths the core records in its session context; neither grants a
   * capability (the core has no file or shell tools). Both required. */
  fx_str home;
  fx_str workspace_root;

  const fx_tool_desc *tools;
  size_t tool_count;

  /* Limits; 0 picks the default shown, and the bounds are enforced. */
  uint32_t max_agent_steps;        /* model calls per turn: 64, at most 10000 */
  uint32_t max_concurrent_tools;   /* unanswered idempotent calls: 16, at most 16 */
  uint32_t max_tool_result_bytes;  /* result text: 64 KiB, at most FX_MAX_TOOL_RESULT_LIMIT */
  uint32_t max_history_turns;      /* turns kept in context: 100, at most 10000 */
  uint32_t max_stop_continuations; /* times stop may continue one turn: 3, at most 64 */
  uint32_t flags;                  /* FX_CONFIG_*; zero */
} fx_config;
#define FX_CONFIG_SIZE_V2 176u

/* ANY. Copies everything cfg names, starts the core thread and sends it
 * initialize. host is copied; host_ctx is kept as a value. On FX_OK *out is
 * the new handle; on any error *out is 0 and nothing is left allocated.
 * Errors: FX_E_INVALID (NULL, struct_size too small, oversized or malformed
 * string, bad tool, bad endpoint, a limit out of bounds), FX_E_UNSUPPORTED
 * (another abi_version, unknown non-zero bytes), FX_E_LIMIT, FX_E_OOM,
 * FX_E_THREAD, FX_E_IO. If initialize then fails in the core, the runtime
 * becomes FAILED with the core's message (fx_runtime_info_get), and every
 * request made meanwhile ends with FX_EVENT_EXITED. */
fx_status fx_runtime_create(const fx_config *cfg, const fx_host *host, void *host_ctx,
                            fx_runtime *out);

/* ANY, BLOCKING; never from a callback. See rule 10 for what the host does
 * first. Waits for calls already inside the handle to return (each is
 * bounded, rule 2), stops callbacks (one already running finishes first),
 * closes the core's input, cancels its turn and fetch, ends every lease and
 * outstanding hook question, waits for the core's threads to finish and frees
 * the runtime, closing its ready fd. If the runtime is PANICKED, or panics
 * while destroy waits, destroy waits only for callbacks already running, then
 * retires the handle: memory and descriptors are abandoned, never freed or
 * closed, so borrows stay readable and a late wake cannot reach a reused
 * descriptor. FX_OK, or FX_E_CLOSED (already destroyed, or never issued). */
fx_status fx_runtime_destroy(fx_runtime rt);

/* ANY. The read end of the runtime's readiness socket (non-blocking,
 * close-on-exec), or -1 if the handle is closed. The core owns it: the host
 * never closes it, and unregisters it before fx_runtime_destroy. It becomes
 * readable when events wait, a fetch is published, the core retires a fetch,
 * space frees after FX_BACKPRESSURE, or the runtime stops. Wake bytes carry
 * no state: the host reads and discards them, then drains every queue to
 * empty and checks every running fetch's disposition. */
int fx_runtime_ready_fd(fx_runtime rt);

typedef int32_t fx_runtime_state;
#define FX_RUNTIME_RUNNING  0
#define FX_RUNTIME_EXITED   1 /* the core thread returned on its own; exit_code 0 or its error */
#define FX_RUNTIME_FAILED   2 /* the C entry stopped the core: initialize failed, or an event or
                               * frame could not be delivered (out of memory); message says what */
#define FX_RUNTIME_PANICKED 3 /* a core thread panicked and is parked; message is its panic */

typedef struct fx_runtime_info {
  uint32_t struct_size;     /* set by the host: >= FX_RUNTIME_INFO_SIZE_V2 */
  fx_runtime_state state;
  int32_t exit_code;
  uint32_t reserved;
  fx_str message;           /* empty while RUNNING; borrowed until fx_runtime_destroy */
} fx_runtime_info;
#define FX_RUNTIME_INFO_SIZE_V2 32u

/* ANY. Lock-free: works whatever the core holds. FX_OK, FX_E_INVALID or
 * FX_E_CLOSED. */
fx_status fx_runtime_info_get(fx_runtime rt, fx_runtime_info *out);

/* ------------------------------------------------------------------------ */
/* Sessions, recovery and images                                            */
/* ------------------------------------------------------------------------ */

typedef struct fx_named_bytes {
  fx_str name;
  fx_bytes bytes;
} fx_named_bytes; /* frozen */

#define FX_OPEN_INPUTS_DURABLE (1u << 0) /* the host stores every prompt and steer before handing it in */
#define FX_OPEN_IMAGE_STORE    (1u << 1) /* the host stores every image under its name (below) */

/*
 * Durable sessions (P-JOURNAL). A session is journaled when fx_host has
 * journal_append. The host owns the store: its head, fencing and order.
 *
 * Appends and acks. The core sends appends in order; the host stores them in
 * order, binds each to the head it expects (fencing out another writer), and
 * acknowledges with fx_journal_ack only what is durable. A barrier append
 * holds the core until it is acknowledged. fx_journal_fail with fenced=true
 * means another writer holds the session; fenced=false means the store
 * failed. Either way the running turn ends with FX_STOP_ERROR, the core
 * appends nothing more, and every later fx_turn_prompt returns FX_E_STATE:
 * the host opens the session again, in a new runtime, from what it stored.
 *
 * Checkpoints. FX_EVENT_CHECKPOINT's bytes are a libfx journal snapshot (its
 * own format, versioned inside; a newer one is refused at open). Its id is
 * the seq of the last journal event it covers. The host stores it only if
 * that seq is the last_seq of the last append it has acknowledged, and
 * stores with it the head that append reached. To open, the host passes the
 * newest stored checkpoint and the events after its seq.
 *
 * Open versus resume. fx_session_open with no journal opens a new session.
 * With a journal it rebuilds the session and reports what it found as
 * FX_EVENT_RECOVERY events, then FX_EVENT_SESSION_OPENED:
 *   FX_RECOVERY_OPEN_TURN   text = the message id of the turn a crash left open;
 *   FX_RECOVERY_LAST_TURN   text = the message id of the last turn that ended;
 *   FX_RECOVERY_CALL        a call the open turn left unanswered: text = tool
 *                           name, text2 = model call id, bytes = input JSON;
 *   FX_RECOVERY_FOLLOW_UP   a queued input: text = its text, text2 = its message id.
 * Nothing runs until the host asks. fx_turn_resume continues the open turn;
 * fx_turn_prompt with the open turn's message id does the same, with the
 * last ended turn's message id it runs nothing (a TURN_START flagged
 * FX_TURN_REPEATED, then its TURN_END), and with any other id it starts a
 * new turn and the open one ends as interrupted. A resumed turn never runs a
 * FX_RECOVERY_CALL again: the model reads that the call may or may not have
 * run, and decides.
 *
 * Images. An image's name is the lowercase hex SHA-256 of its raw encoded
 * bytes (64 characters). With FX_OPEN_IMAGE_STORE the host stores every
 * image it hands in (prompt, steer and tool result) under its name before
 * handing it in, and the journal names images instead of holding their
 * bytes. Opening a journal that names images the core does not hold ends
 * with FX_EVENT_IMAGES_NEEDED listing every missing name at once; the host
 * stages them with fx_session_stage_images in as many batches as it needs
 * and calls fx_session_open again with the same arguments.
 */
typedef struct fx_session_open_args {
  uint32_t struct_size;  /* >= FX_SESSION_OPEN_ARGS_SIZE_V2 */
  uint32_t flags;        /* FX_OPEN_* */
  fx_str session_id;     /* FX_MAX_SESSION_ID; empty: the core makes one */
  fx_str journal_json;   /* stored journal events after the checkpoint, a JSON array, oldest first; may be empty */
  fx_bytes checkpoint;   /* the stored checkpoint's bytes, or empty */
} fx_session_open_args;
#define FX_SESSION_OPEN_ARGS_SIZE_V2 56u

/* WORKER. Opens the runtime's one session. Everything args names is copied.
 * Terminal event for *request: FX_EVENT_SESSION_OPENED,
 * FX_EVENT_IMAGES_NEEDED, or FX_EVENT_REQUEST_DONE with an error. FX_E_STATE
 * if a session is open or opening; FX_E_TOO_LARGE if the journal and
 * checkpoint can never fit; FX_BACKPRESSURE. BUILD: a journal or checkpoint
 * returns FX_E_UNSUPPORTED until milestone 2b (resume). */
fx_status fx_session_open(fx_runtime rt, const fx_session_open_args *args, uint64_t *request);

/* WORKER. Stages one batch of named images (at most FX_MAX_STAGE_BATCH and
 * FX_MAX_STAGE_BATCH_BYTES; copied) for the next fx_session_open. Terminal
 * event: FX_EVENT_REQUEST_DONE (FX_OK staged, or an error such as a name
 * that does not match its bytes). FX_E_STATE once a session is open.
 * BUILD: FX_E_UNSUPPORTED until milestone 2b (P-IMAGES). */
fx_status fx_session_stage_images(fx_runtime rt, const fx_named_bytes *images, size_t count,
                                  uint64_t *request);

/* ------------------------------------------------------------------------ */
/* Turns                                                                    */
/* ------------------------------------------------------------------------ */

typedef struct fx_image {
  fx_str mime;     /* image/png, image/jpeg, image/webp or image/gif */
  fx_bytes data;   /* raw encoded bytes, never base64; FX_MAX_IMAGE_BYTES */
} fx_image; /* frozen */

typedef struct fx_input {
  fx_str text;
  const fx_image *images;  /* FX_MAX_PROMPT_IMAGES */
  size_t image_count;
} fx_input; /* frozen: a richer input is a new struct and function */

/* WORKER. Starts a turn with input (copied; the core keeps it as history).
 * message_id (FX_MAX_MESSAGE_ID) names the input in the journal: a retry
 * with the same id is recognised (see "Open versus resume"), never run
 * twice. On FX_OK *turn is the new turn and its FX_EVENT_TURN_START is
 * queued. FX_E_STATE before the session is open, while a turn runs (steer it
 * instead) or after a journal failure; FX_E_TOO_LARGE; FX_BACKPRESSURE.
 * BUILD: images return FX_E_UNSUPPORTED until milestone 2b (P-IMAGES). */
fx_status fx_turn_prompt(fx_runtime rt, const fx_input *input, fx_str message_id, uint64_t *turn);

/* WORKER. Continues the turn a crash left open (FX_RECOVERY_OPEN_TURN), with
 * no new input. FX_E_STATE when there is none. BUILD: FX_E_UNSUPPORTED until
 * milestone 2b (resume). */
fx_status fx_turn_resume(fx_runtime rt, uint64_t *turn);

/* WORKER. Steers running turn `turn` with input (copied), libfx's way: it
 * cuts a streaming reply short, waits for a running tool, and joins a turn
 * waiting on the model, showing as FX_EVENT_STEER_JOINED. FX_STALE when
 * `turn` is not the running turn: nothing was queued, and the host sends the
 * input as a prompt instead. On FX_OK the terminal event for *request is
 * FX_EVENT_REQUEST_DONE: FX_OK queued for the turn, FX_STALE the turn ended
 * before the core read it, or an error. A steer still queued when its turn
 * ends comes back as FX_EVENT_STEER_RETURNED before that turn's TURN_END, to
 * be sent as a prompt. Text is at most FX_MAX_STEER_TEXT. A steer with images
 * cannot join a turn: the turn ends at its next safe point and the steer
 * comes back as FX_EVENT_STEER_RETURNED (the handoff). BUILD: images return
 * FX_E_UNSUPPORTED until milestone 2b (P-IMAGES). */
fx_status fx_turn_steer(fx_runtime rt, uint64_t turn, const fx_input *input, fx_str message_id,
                        uint64_t *request);

/* WORKER. Withdraws a steer of turn `turn` that has not joined. FX_STALE
 * when `turn` is not the running turn. On FX_OK, FX_EVENT_REQUEST_DONE for
 * *request says FX_OK (withdrawn) or FX_STALE (it had joined, or the turn
 * ended). */
fx_status fx_turn_withdraw(fx_runtime rt, uint64_t turn, fx_str message_id, uint64_t *request);

/* ANY. Cancels running turn `turn`: its fetch is retired at once (a host
 * waiting on its response headers sees FX_FETCH_RETIRED and a wake), its
 * unanswered tool calls get tool_cancel, and it ends with FX_EVENT_TURN_END
 * (FX_STOP_CANCELLED) unless it was already ending. FX_OK, or FX_STALE when
 * `turn` is not the running turn (nothing changed: a late cancel never
 * reaches a later turn). Never FX_BACKPRESSURE. */
fx_status fx_turn_cancel(fx_runtime rt, uint64_t turn);

/* WORKER. Asks for a checkpoint of the journaled session. Terminal event for
 * *request: FX_EVENT_CHECKPOINT, or FX_EVENT_REQUEST_DONE with an error.
 * FX_E_STATE when the session is not open or not journaled. BUILD:
 * FX_E_UNSUPPORTED until milestone 2b (resume). */
fx_status fx_checkpoint(fx_runtime rt, uint64_t *request);

/* ------------------------------------------------------------------------ */
/* Events                                                                   */
/* ------------------------------------------------------------------------ */

typedef uint32_t fx_event_kind;
#define FX_EVENT_SESSION_OPENED   1  /* request; text = session id; flags FX_OPENED_*; id = turns in history */
#define FX_EVENT_IMAGES_NEEDED    2  /* request; names = every missing image name */
#define FX_EVENT_REQUEST_DONE     3  /* request; status; text = error message */
#define FX_EVENT_TURN_START       4  /* turn; text = message id; flags FX_TURN_* */
#define FX_EVENT_TEXT_DELTA       5  /* turn; text = reply text */
#define FX_EVENT_THOUGHT_DELTA    6  /* turn; text = reasoning text */
#define FX_EVENT_STEER_JOINED     7  /* turn; request = the steer's; text = steer text; text2 = its message id */
#define FX_EVENT_TOOL_START       8  /* turn; text = tool name; text2 = model call id */
#define FX_EVENT_TOOL_END         9  /* turn; text = tool name; text2 = model call id; status FX_OK or FX_E_FAILED */
#define FX_EVENT_USAGE           10  /* turn; usage.context_used and context_size */
#define FX_EVENT_COMPACTED       11  /* turn; the core compacted the context */
#define FX_EVENT_TURN_END        12  /* turn; code = FX_STOP_*; usage = the turn's token totals */
#define FX_EVENT_CHECKPOINT      13  /* request; bytes = checkpoint; id = the last journal seq it covers */
#define FX_EVENT_ERROR           14  /* turn or 0; status; text = message */
#define FX_EVENT_EXITED          15  /* the last event; code = fx_runtime_state; status = exit code;
                                      * text = the runtime's message; flags FX_EXITED_* */
#define FX_EVENT_STEER_RETURNED  16  /* turn; request = the steer's; text2 = its message id: send it as a prompt */
#define FX_EVENT_RECOVERY        17  /* request; code = FX_RECOVERY_*; see "Open versus resume" */

#define FX_OPENED_JOURNALED  (1u << 0) /* the session is journaled */
#define FX_OPENED_RESUMABLE  (1u << 1) /* a turn the crash left open can resume */
#define FX_OPENED_YIELDED    (1u << 2) /* the open turn stopped at the host's yield, not in a crash */

#define FX_TURN_REPEATED (1u << 0) /* the message id named a turn that already ended: nothing runs */
#define FX_TURN_RESUMED  (1u << 1) /* this continues the turn a crash left open */

#define FX_EXITED_EVENTS_LOST (1u << 0) /* the core failed holding the event queue: queued events are lost */

#define FX_RECOVERY_OPEN_TURN 1
#define FX_RECOVERY_LAST_TURN 2
#define FX_RECOVERY_CALL      3
#define FX_RECOVERY_FOLLOW_UP 4

typedef int32_t fx_stop_reason;
#define FX_STOP_END_TURN      0
#define FX_STOP_MAX_TOKENS    1
#define FX_STOP_MAX_STEPS     2 /* the step limit ended the turn */
#define FX_STOP_REFUSAL       3
#define FX_STOP_CANCELLED     4
#define FX_STOP_YIELDED       5 /* a journaled turn stopped at its yield point and stays open */
#define FX_STOP_ERROR         6 /* the turn failed; an FX_EVENT_ERROR came before */

/* Zero when unknown. Frozen: part of fx_event's v2 layout. */
typedef struct fx_usage {
  uint64_t input_tokens;
  uint64_t output_tokens;
  uint64_t cache_read_tokens;
  uint64_t cache_write_tokens;
  uint64_t reasoning_tokens;
  uint64_t context_used;
  uint64_t context_size;
} fx_usage;

/* A typed event. Fields a kind does not use are zero. Every pointer in it is
 * borrowed until the next fx_events_drain on the runtime that returns
 * FX_OK, or fx_runtime_destroy. */
typedef struct fx_event {
  fx_event_kind kind;
  uint32_t flags;
  uint64_t turn;     /* the turn id, or 0 */
  uint64_t request;  /* the request id, or 0 */
  uint64_t id;
  fx_status status;
  int32_t code;
  fx_str text;
  fx_str text2;
  fx_bytes bytes;
  const fx_str *names;
  size_t name_count;
  fx_usage usage;
} fx_event;
#define FX_EVENT_SIZE_V2 160u

/* WORKER. Moves up to cap queued events, oldest first, into buf, an array of
 * cap elements event_size bytes apart (sizeof(fx_event) as the host compiled
 * it: at least FX_EVENT_SIZE_V2, a multiple of 8); the core writes its fields
 * and zeroes the rest of each element. *count is how many it wrote. Every
 * call that returns FX_OK, cap 0 and *count 0 included, ends the borrows of
 * the previous drain. After the runtime stops, the drain delivers the events
 * queued before it stopped, then FX_EVENT_EXITED once, then nothing; if the
 * core panicked holding the event queue, it delivers FX_EVENT_EXITED with
 * FX_EXITED_EVENTS_LOST at once. Never blocks on the core. FX_OK,
 * FX_E_INVALID, FX_E_CLOSED or FX_BUSY (on a stopped runtime too). A text
 * delta costs the core one copy into its event arena; a drain that takes
 * everything queued costs no copy. */
fx_status fx_events_drain(fx_runtime rt, fx_event *buf, size_t cap, size_t event_size,
                          size_t *count);

/* ------------------------------------------------------------------------ */
/* Tool results                                                             */
/* ------------------------------------------------------------------------ */

typedef struct fx_tool_output {
  fx_str text;               /* what the model reads; at most max_tool_result_bytes */
  const fx_image *images;    /* frames the model sees, raw bytes */
  size_t image_count;
  bool is_error;             /* text is an agent-oriented error */
} fx_tool_output; /* frozen: a richer output is a new struct and function */

/* ANY. Answers tool call `call`, ending its lease (rule 9). out is copied
 * once, into memory the core owns. An answer consumes the call exactly when
 * it returns FX_OK or FX_E_TOO_LARGE (the model then reads that the result
 * exceeded the limit). FX_STALE: already answered, nothing changed (a
 * cancelled call's answer is FX_OK: it ends the lease, and the core drops
 * the result). FX_E_INVALID: a NULL or malformed output, or an id never
 * issued; the call stays unanswered. A runtime that is not running answers
 * FX_E_FAILED; on a panicked runtime the lease's memory stays readable.
 * BUILD: images return FX_E_UNSUPPORTED (the call stays unanswered) until
 * milestone 2b (P-IMAGES). */
fx_status fx_tool_result(fx_runtime rt, uint64_t call, const fx_tool_output *out);

/* ------------------------------------------------------------------------ */
/* Model HTTP (the fetch bridge)                                            */
/* ------------------------------------------------------------------------ */

typedef struct fx_header {
  fx_str name;
  fx_str value;
} fx_header; /* frozen */

typedef uint32_t fx_fetch_purpose;
#define FX_FETCH_MODEL   1u /* a model request (POST) */
#define FX_FETCH_CATALOG 2u /* the provider's model catalog (GET) */

typedef struct fx_fetch_request {
  uint32_t struct_size;      /* set by the host: >= FX_FETCH_REQUEST_SIZE_V2 */
  fx_fetch_purpose purpose;
  uint64_t fetch;
  uint64_t turn;             /* the turn it serves, or 0 (setup outside a turn) */
  fx_str provider;           /* the provider id: gateway, codex, grok, or the configured name */
  fx_str method;
  fx_str url;
  const fx_header *headers;  /* credential headers stripped (rule 13) */
  size_t header_count;
  fx_bytes body;
} fx_fetch_request;
#define FX_FETCH_REQUEST_SIZE_V2 104u

/* WORKER. Takes the request the core published, if any (at most one is
 * published at a time). On FX_OK *out describes it; its pointers are
 * borrowed until the next fx_fetch_take on the runtime that returns FX_OK
 * or FX_EMPTY, or fx_runtime_destroy. FX_EMPTY when none waits. The host
 * sends it (adding credentials per rule 13) and streams the answer back with
 * the calls below, in order: fx_fetch_status once, then fx_fetch_push any
 * number of times, then fx_fetch_finish; fx_fetch_finish(ok=false) may come
 * at any point, before the status included, for a transport failure. */
fx_status fx_fetch_take(fx_runtime rt, fx_fetch_request *out);

/* ANY. The response status line arrived. retry_after_s is Retry-After in
 * whole seconds, or -1. FX_OK; FX_STALE (the core retired the fetch: abort
 * the HTTP request); FX_E_STATE (status already given); FX_E_INVALID. */
fx_status fx_fetch_status(fx_runtime rt, uint64_t fetch, uint16_t status, int32_t retry_after_s);

/* ANY. Appends response bytes (copied). FX_OK; FX_STALE (retired: stop);
 * FX_BACKPRESSURE (FX_MAX_FETCH_QUEUE unread: retry the same bytes after the
 * next wake); FX_E_TOO_LARGE (len > FX_MAX_FETCH_CHUNK: split it); FX_E_STATE
 * (before fx_fetch_status). */
fx_status fx_fetch_push(fx_runtime rt, uint64_t fetch, const uint8_t *data, size_t len);

/* ANY. The response ended: ok true for a clean end, false for a transport
 * failure (allowed before the status). FX_OK or FX_STALE. */
fx_status fx_fetch_finish(fx_runtime rt, uint64_t fetch, bool ok);

typedef int32_t fx_fetch_disposition;
#define FX_FETCH_RETIRED  0 /* the core gave it up (cancel, steer, deadline, stop): abort the HTTP request */
#define FX_FETCH_ACTIVE   1 /* the core still reads it */
#define FX_FETCH_CONSUMED 2 /* the core has all it needs: drain the rest quietly */

/* ANY. Where the core stands on fetch. Checked by the host after every wake
 * while the HTTP request runs; the core wakes the ready fd whenever it
 * retires a fetch. Returns FX_FETCH_RETIRED for a closed or stopped runtime,
 * an unknown fetch, or a lock contended past the bound. */
fx_fetch_disposition fx_fetch_disposition_get(fx_runtime rt, uint64_t fetch);

/* ------------------------------------------------------------------------ */
/* Journal acknowledgements                                                 */
/* ------------------------------------------------------------------------ */

/* ANY. The store holds every append up to and including `append` durably
 * (acknowledging a later one acknowledges the earlier). Releases barriers
 * waiting on them. FX_OK; FX_STALE (already acknowledged, or after a
 * journal failure); FX_E_INVALID (an id the core never sent). Never
 * FX_BACKPRESSURE. */
fx_status fx_journal_ack(fx_runtime rt, uint64_t append);

/* ANY. The store refused `append` (reason copied; empty for a default
 * message). See "Durable sessions" for fenced. FX_OK; FX_STALE (already
 * acknowledged, or a failure was already reported); FX_E_INVALID. */
fx_status fx_journal_fail(fx_runtime rt, uint64_t append, fx_str reason, bool fenced);

/* ------------------------------------------------------------------------ */
/* Hook answers                                                             */
/* ------------------------------------------------------------------------ */

typedef int32_t fx_hook_answer_kind;
#define FX_ANSWER_ALLOW     0  /* PRE_TOOL: run unchanged. STOP: end the turn */
#define FX_ANSWER_BLOCK     1  /* PRE_TOOL: do not run; text is the reason the model reads */
#define FX_ANSWER_REWRITE   2  /* PRE_TOOL: run with text as the new input JSON */
#define FX_ANSWER_CONTINUE  3  /* STOP: continue the turn; text is the message the model reads */

/* ANY. Answers a question the hook returned FX_HOOK_PENDING for, ending its
 * lease (rule 9). text (copied) is the block reason, the rewritten input or
 * the continuation message; empty for ALLOW. FX_OK; FX_STALE (answered
 * already, or its turn was cancelled: the answer is dropped, the lease ends);
 * FX_E_INVALID (an id never issued, or a kind the question does not take:
 * the question stays open). BUILD: no question is asked until milestone 2b
 * (P-STOP, P-PRETOOL), so every id is FX_E_INVALID. */
fx_status fx_hook_answer(fx_runtime rt, uint64_t question, fx_hook_answer_kind kind, fx_str text);

/* ------------------------------------------------------------------------ */
/* Testing only                                                             */
/* ------------------------------------------------------------------------ */

/* Fault points for the core's own containment tests (P-PANIC). A host never
 * arms one outside tests. */
#define FX_FAULT_FETCH_LOCKED   1u /* a core thread, holding the fetch bridge lock */
#define FX_FAULT_EVENT_LOCKED   2u /* a core thread, holding the event queue lock */
#define FX_FAULT_STREAMING      3u /* a core thread, turning a streamed text delta into an event */
#define FX_FAULT_TOOL_RESULT    4u /* inside fx_tool_result on the host's thread, holding the runtime lock */
#define FX_FAULT_JOURNAL_APPEND 5u /* a core thread, sending a journal append, holding the runtime lock */
#define FX_FAULT_TOOL_CALL      6u /* a core thread, about to make a tool call */

/* ANY. Arms `point` on runtime rt: the (skip + 1)-th time a thread reaches it
 * from now, that thread panics there. FX_OK, FX_E_INVALID, FX_E_CLOSED. */
fx_status fx_test_arm_fault(fx_runtime rt, uint32_t point, uint32_t skip);

/* ANY. Sets the generation slot `slot` (0-based) gives its next runtime, so
 * tests can reach generation exhaustion. Only on a free slot: FX_OK,
 * FX_E_INVALID (bad slot or generation, or the slot is in use). */
fx_status fx_test_set_slot_generation(uint32_t slot, uint64_t generation);

#ifdef __cplusplus
}
#endif

#endif /* FX_CORE_H */
