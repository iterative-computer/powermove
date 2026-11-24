/*
 * fx_core.h: the C ABI of Powermove's libfx core (patch P-CAPI).
 *
 * Powermove notice: new file, not in upstream fx. Copyright the Powermove
 * authors; licensed under Apache-2.0 like the rest of this vendored tree.
 * Implemented by src/capi_core_main.zig. See POWERMOVE.md.
 *
 * One runtime is one agent session on its own core thread. The host (Rust,
 * the `pm-aero` worker) drives it with the non-blocking calls below and
 * drains what the core produces: typed events, model HTTP requests, tool
 * calls, journal appends and hook questions.
 *
 * RULES THAT HOLD FOR EVERY FUNCTION
 *
 * Threads. No function may be called on the process's main thread. Every
 * function returns without waiting for the core, except fx_runtime_destroy,
 * which joins the core thread. Functions marked WORKER are called by one
 * thread at a time per runtime (the thread that drains it; it may change
 * between calls if the host serialises them). Functions marked ANY may be
 * called from any thread, concurrently with each other and with WORKER calls.
 * fx_host callbacks run on core-owned threads (the runtime's thread or one of
 * its prompt or tool workers), never on a host thread, and never while a host
 * call into this API is on the stack of that thread.
 *
 * Handles. fx_runtime is an opaque generation-tagged 64-bit value, never a
 * pointer. 0 is never a valid handle. Once fx_runtime_destroy has started on a
 * handle, every call naming it returns FX_E_CLOSED (or 0 / -1 where a
 * function returns a count or a descriptor), even after the slot is reused by
 * a new runtime, whose handle carries a new generation. Ids inside a runtime
 * (requests, turns, tool calls, fetches, journal appends, hook questions) are
 * 64-bit, start at 1 and never repeat within that runtime.
 *
 * Failed runtimes. A runtime fails when its core panics (P-PANIC), when its
 * output transport fails, or when the core exits on its own. After that every
 * call except fx_runtime_info, fx_events_drain and fx_runtime_destroy returns
 * FX_E_FAILED; the drain still delivers the events queued before the failure
 * and then one FX_EVENT_EXITED. A runtime whose core thread panicked is never
 * joined and its memory is never freed: fx_runtime_destroy only retires the
 * handle (FX_OK), so a pointer the host still holds into it stays readable.
 *
 * Strings and bytes. fx_str is UTF-8 with an explicit length and no NUL
 * terminator; fx_bytes is raw bytes. A NULL pointer is allowed only with
 * length 0. What the host hands in is either copied before the call returns
 * ("copied") or read only during the call ("borrowed for the call"); the core
 * never keeps a host pointer. What the core hands out is borrowed, for as long
 * as each function or callback states; the host never frees it.
 *
 * Statuses. Every fx_status function returns FX_OK or one of the codes below.
 * Non-negative codes other than FX_OK are outcomes, not errors.
 */
#ifndef FX_CORE_H
#define FX_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Bumped on any incompatible change. Additive changes (new event kinds, new
 * trailing struct fields guarded by struct_size) keep it. */
#define FX_CORE_ABI_VERSION 1u

typedef int32_t fx_status;
#define FX_OK              0  /* applied */
#define FX_EMPTY           1  /* nothing waiting (fx_fetch_take) */
#define FX_STALE           2  /* the id is no longer current; nothing changed */
#define FX_BACKPRESSURE    3  /* a bounded queue is full; retry after the next wake */
#define FX_E_INVALID      -1  /* bad argument: NULL, oversized, malformed, unknown id */
#define FX_E_CLOSED       -2  /* the handle was destroyed (or never existed) */
#define FX_E_FAILED       -3  /* the runtime failed; see fx_runtime_info */
#define FX_E_LIMIT        -4  /* the process-wide limit of 64 runtimes is reached */
#define FX_E_OOM          -5  /* the core could not allocate */
#define FX_E_THREAD       -6  /* the core could not start a thread */
#define FX_E_IO           -7  /* the readiness socket could not be made */
#define FX_E_STATE        -8  /* not valid now (prompt before open, second open) */
#define FX_E_UNAVAILABLE  -9  /* this build has no core (built without Zig) */
#define FX_E_UNSUPPORTED -10  /* this build does not implement the call yet */

typedef uint64_t fx_runtime;

typedef struct fx_str {
  const char *ptr;
  size_t len;
} fx_str;

typedef struct fx_bytes {
  const uint8_t *ptr;
  size_t len;
} fx_bytes;

/* ------------------------------------------------------------------------ */
/* Version and availability                                                 */
/* ------------------------------------------------------------------------ */

/* ANY. Returns FX_CORE_ABI_VERSION of the linked core. The stub built without
 * Zig returns 0; every other function of the stub returns FX_E_UNAVAILABLE
 * (or 0 / -1 for count and descriptor functions). */
uint32_t fx_core_abi_version(void);

/* ------------------------------------------------------------------------ */
/* Host callbacks                                                           */
/* ------------------------------------------------------------------------ */

#define FX_TOOL_IDEMPOTENT (1u << 0) /* may run beside other idempotent calls */
#define FX_TOOL_WRITES     (1u << 1) /* changes state: runs alone, never re-run after a crash */

/* Verdicts a hook callback returns at once. */
typedef int32_t fx_hook_verdict;
#define FX_HOOK_ALLOW    0  /* let the tool run / let the turn end */
#define FX_HOOK_PENDING  1  /* the host answers later with fx_hook_answer */

/* Answers given later through fx_hook_answer. */
typedef int32_t fx_hook_answer_kind;
#define FX_ANSWER_ALLOW     0  /* pre_tool: run unchanged. stop: end the turn */
#define FX_ANSWER_BLOCK     1  /* pre_tool: do not run; text is the reason the model sees */
#define FX_ANSWER_REWRITE   2  /* pre_tool: run with text as the new input JSON */
#define FX_ANSWER_CONTINUE  3  /* stop: continue the turn; text is the message the model sees */

/*
 * Every callback runs on a core-owned thread and must return quickly: it may
 * only record or enqueue, never do the work, block, or call back into this API
 * on the same thread. The pointer arguments of a callback are borrowed for the
 * call unless its comment says longer. host_ctx is the value given to
 * fx_runtime_create; the core never dereferences it. A NULL callback means the
 * host does not take part: no tool can be called (tool_call), cancels are not
 * reported (tool_cancel), the session is not journaled (journal_append), every
 * tool is allowed (pre_tool), every turn may end (stop). The host learns that
 * something waits only from the ready fd (fx_runtime_ready_fd).
 */
typedef struct fx_host {
  /* Size of this struct as the host compiled it; the core reads only the
   * fields that fit. */
  uint32_t struct_size;
  uint32_t reserved;

  /* The model called a host tool. call is the id fx_tool_result answers.
   * name, input_json (the model's own JSON text) and model_call_id stay valid
   * until fx_tool_result for this call has returned, or until the runtime is
   * destroyed, whichever is first; the host may parse input_json in place
   * from another thread in that window. flags are the tool's FX_TOOL_* flags.
   * Up to max_concurrent_tools idempotent calls may be outstanding at once; a
   * call with FX_TOOL_WRITES is only made when no other call is outstanding. */
  void (*tool_call)(void *host_ctx, uint64_t call, fx_str name, fx_str input_json,
                    fx_str model_call_id, uint32_t flags);

  /* The turn that made call was cancelled or steered past it. The host stops
   * the work if it can, and must still answer with fx_tool_result (whose
   * result the core drops) to release the call. */
  void (*tool_cancel)(void *host_ctx, uint64_t call);

  /* The core asks the host to store journal events. events_json is a JSON
   * array of libfx journal events in seq order, borrowed for the call: the
   * host copies it. Appends arrive in order with increasing ids. barrier says
   * the core waits for this append (and every earlier one) to be acknowledged
   * with fx_journal_ack before it goes on. The host owns the store's cursor
   * and fencing (P-JOURNAL). */
  void (*journal_append)(void *host_ctx, uint64_t append, fx_str events_json, bool barrier);

  /* Before a host tool runs (fx's PreToolUse). Return FX_HOOK_ALLOW to let it
   * run now, or FX_HOOK_PENDING and answer `question` later with
   * fx_hook_answer (ALLOW, BLOCK or REWRITE). name and input_json as in
   * tool_call, but borrowed only until the question is answered. */
  fx_hook_verdict (*pre_tool)(void *host_ctx, uint64_t question, uint64_t call,
                              fx_str name, fx_str input_json);

  /* The model ended its reply; before the turn ends (fx's Stop, the
   * completion gate, P-STOP). Return FX_HOOK_ALLOW to end the turn, or
   * FX_HOOK_PENDING and answer `question` later (ALLOW or CONTINUE).
   * final_text is the reply, borrowed until the question is answered.
   * continuations is how many times this turn was already continued; once it
   * reaches the runtime's max_stop_continuations the core ends the turn
   * without asking. */
  fx_hook_verdict (*stop)(void *host_ctx, uint64_t question, fx_str final_text,
                          uint32_t continuations);
} fx_host;

/* ------------------------------------------------------------------------ */
/* Runtimes                                                                 */
/* ------------------------------------------------------------------------ */

typedef struct fx_tool_desc {
  fx_str name;              /* [A-Za-z0-9_-]{1,64}, unique in the runtime */
  fx_str description;       /* what the model reads */
  fx_str input_schema_json; /* a JSON Schema object */
  uint32_t flags;           /* FX_TOOL_* */
  uint32_t reserved;
} fx_tool_desc;

typedef struct fx_config {
  uint32_t struct_size;     /* sizeof(fx_config) as the host compiled it */
  uint32_t abi_version;     /* FX_CORE_ABI_VERSION, or the create fails with FX_E_INVALID */

  /* Model selection. provider: "gateway" (empty means gateway), "codex",
   * "grok", or the name of a provider configured_providers_json defines.
   * model and effort may be empty (the core's defaults). */
  fx_str provider;
  fx_str model;
  fx_str effort;
  fx_str configured_providers_json; /* settings-style providers object, no credentials; empty if none */
  /* The model endpoint the core builds requests for: empty for the provider's
   * canonical URL, else http on 127.0.0.1, localhost or [::1] with a port
   * (local fakes and tests only). Credentials never enter the core: the host
   * adds them to each request at its fetch edge. The core may also ask for
   * the provider's model catalog (a GET to its canonical URL, whatever
   * endpoint_url says); the host decides what reaches the network and may
   * answer any request itself (a 404 makes the core use its built-in model
   * capabilities). */
  fx_str endpoint_url;

  /* The system instructions the model sees (the topic map). */
  fx_str instructions;
  /* Identity paths the core records in its session context; neither grants a
   * capability (the core has no file or shell tools). */
  fx_str home;
  fx_str workspace_root;

  const fx_tool_desc *tools;
  size_t tool_count;

  /* Limits; 0 picks the core's default (shown). */
  uint32_t max_agent_steps;        /* model calls per turn (64) */
  uint32_t max_concurrent_tools;   /* idempotent calls at once (16) */
  uint32_t max_tool_result_bytes;  /* text the model sees per result (64 KiB) */
  uint32_t max_history_turns;      /* turns kept in context (100) */
  uint32_t max_stop_continuations; /* times stop may continue one turn (3) */
  uint32_t reserved;
} fx_config;

/* ANY (not main). Copies everything cfg names, starts the core thread and
 * sends it initialize. host is copied; host_ctx is kept as a value. On FX_OK
 * *out is the new handle; on any error *out is 0 and nothing is left
 * allocated. Errors: FX_E_INVALID (NULL, wrong abi_version or struct_size,
 * oversized string, bad tool, bad endpoint), FX_E_LIMIT, FX_E_OOM,
 * FX_E_THREAD, FX_E_IO. */
fx_status fx_runtime_create(const fx_config *cfg, const fx_host *host, void *host_ctx,
                            fx_runtime *out);

/* ANY (not main), BLOCKING. Closes the core's input, cancels its turn and
 * fetch, answers every outstanding tool call and hook question as cancelled,
 * joins the core thread and frees the runtime, its ready fd included. Pointers
 * the core handed out become invalid when it returns. Waits for concurrent
 * calls on the same handle to return first. Returns FX_OK, or FX_E_CLOSED if
 * the handle is already destroyed or unknown. On a failed runtime it only
 * retires the handle (see "Failed runtimes"). */
fx_status fx_runtime_destroy(fx_runtime rt);

/* ANY. The read end of the runtime's readiness socket (non-blocking,
 * close-on-exec), or -1 if the handle is closed. The core owns it and closes
 * it in fx_runtime_destroy: the host never closes it. It becomes readable
 * when events, a fetch, response space or an exit wait. Wake bytes carry no
 * state: the host reads and discards them, then drains every queue to empty. */
int fx_runtime_ready_fd(fx_runtime rt);

typedef int32_t fx_runtime_state;
#define FX_RUNTIME_RUNNING 0
#define FX_RUNTIME_EXITED  1 /* the core thread returned; exit_code says how */
#define FX_RUNTIME_FAILED  2 /* the core panicked; message says where */

typedef struct fx_runtime_info {
  fx_runtime_state state;
  int32_t exit_code;
  fx_str message; /* panic message, or empty; valid until fx_runtime_destroy */
} fx_runtime_info;

/* ANY. Fills *out. FX_OK, or FX_E_CLOSED. Works on a failed runtime. */
fx_status fx_runtime_info_get(fx_runtime rt, fx_runtime_info *out);

/* ------------------------------------------------------------------------ */
/* Sessions and turns                                                       */
/* ------------------------------------------------------------------------ */

typedef struct fx_named_bytes {
  fx_str name;
  fx_bytes bytes;
} fx_named_bytes;

typedef struct fx_session_open_args {
  uint32_t struct_size;
  uint32_t reserved;
  fx_str session_id;     /* empty: the core makes one */
  /* Resume (both optional). journal_json: the stored journal events, a JSON
   * array, oldest first. checkpoint: the last checkpoint's bytes. */
  fx_str journal_json;
  fx_bytes checkpoint;
  /* Images the journal names, staged for the open (see
   * FX_EVENT_IMAGES_NEEDED). At most 8, totalling at most 8 MiB. */
  const fx_named_bytes *images;
  size_t image_count;
  bool inputs_durable;   /* the host stores prompts before sending them */
} fx_session_open_args;

/* WORKER. Opens the runtime's one session: new, or resumed from the journal
 * and checkpoint given. Everything args names is copied. The outcome arrives
 * as FX_EVENT_SESSION_OPENED, FX_EVENT_IMAGES_NEEDED (open again with those
 * images staged) or FX_EVENT_REQUEST_DONE with an error, all carrying
 * *request. FX_E_STATE if a session is already open. */
fx_status fx_session_open(fx_runtime rt, const fx_session_open_args *args, uint64_t *request);

typedef struct fx_image {
  fx_str mime;     /* image/png, image/jpeg, image/webp or image/gif */
  fx_bytes data;   /* raw encoded bytes, never base64 */
} fx_image;

typedef struct fx_input {
  fx_str text;
  const fx_image *images;
  size_t image_count;
} fx_input;

/* WORKER. Starts a turn. input is copied (the core keeps it as history).
 * message_id ([A-Za-z0-9._:-]{1,128}) names the prompt in the journal: a
 * retry with the same id after a crash is recognised, never run twice. The
 * turn's events carry *turn, starting with FX_EVENT_TURN_START (queued before
 * this returns) and ending with FX_EVENT_TURN_END. FX_E_STATE before the
 * session is open or while a turn runs (steer it instead). */
fx_status fx_turn_prompt(fx_runtime rt, const fx_input *input, fx_str message_id, uint64_t *turn);

/* WORKER. Steers the running turn with text (copied), libfx's way: a steer
 * cuts a streaming reply short, waits for a running tool, and joins a turn
 * waiting on the model. It shows in the turn as FX_EVENT_STEER_JOINED with its
 * message_id. Accepted or refused: FX_EVENT_REQUEST_DONE with *request. */
fx_status fx_turn_steer(fx_runtime rt, fx_str text, fx_str message_id, uint64_t *request);

/* WORKER. Withdraws a steer that has not joined yet. FX_EVENT_REQUEST_DONE
 * with *request says FX_OK (withdrawn) or FX_STALE (it had already joined). */
fx_status fx_turn_withdraw(fx_runtime rt, fx_str message_id, uint64_t *request);

/* ANY. Cancels the running turn: its fetch is aborted, its tool calls get
 * tool_cancel, and it ends with FX_EVENT_TURN_END (FX_STOP_CANCELLED). FX_OK
 * also when no turn runs. */
fx_status fx_turn_cancel(fx_runtime rt);

/* WORKER. Asks for a checkpoint of the session (libfx's kernel checkpoint).
 * It arrives as FX_EVENT_CHECKPOINT with *request; its bytes are what
 * fx_session_open_args.checkpoint takes back. */
fx_status fx_checkpoint(fx_runtime rt, uint64_t *request);

/* ------------------------------------------------------------------------ */
/* Events                                                                   */
/* ------------------------------------------------------------------------ */

typedef uint32_t fx_event_kind;
#define FX_EVENT_SESSION_OPENED   1  /* request; text = session id; flags: FX_OPENED_* */
#define FX_EVENT_IMAGES_NEEDED    2  /* request; names = image names to stage */
#define FX_EVENT_REQUEST_DONE     3  /* request; status; text = error message */
#define FX_EVENT_TURN_START       4  /* turn; text = message id */
#define FX_EVENT_TEXT_DELTA       5  /* turn; text = reply text */
#define FX_EVENT_THOUGHT_DELTA    6  /* turn; text = reasoning text */
#define FX_EVENT_STEER_JOINED     7  /* turn; text = steer text; text2 = its message id */
#define FX_EVENT_TOOL_START       8  /* turn; text = tool name; text2 = model call id */
#define FX_EVENT_TOOL_END         9  /* turn; text = tool name; text2 = model call id; status: FX_OK or FX_E_FAILED */
#define FX_EVENT_USAGE           10  /* turn; usage */
#define FX_EVENT_COMPACTED       11  /* turn; the core compacted the context */
#define FX_EVENT_TURN_END        12  /* turn; code = FX_STOP_*; usage */
#define FX_EVENT_CHECKPOINT      13  /* request; bytes = checkpoint */
#define FX_EVENT_ERROR           14  /* turn or 0; status; text = message */
#define FX_EVENT_EXITED          15  /* the core exited or failed; last event */

#define FX_OPENED_RESUMABLE (1u << 0) /* a turn the crash left open can resume (prompt continues it) */

typedef int32_t fx_stop_reason;
#define FX_STOP_END_TURN      0
#define FX_STOP_MAX_TOKENS    1
#define FX_STOP_MAX_STEPS     2 /* the step limit ended the turn */
#define FX_STOP_REFUSAL       3
#define FX_STOP_CANCELLED     4
#define FX_STOP_YIELDED       5 /* a journaled turn stopped at its yield point and stays open */
#define FX_STOP_ERROR         6 /* the turn failed; an FX_EVENT_ERROR came before */

/* Token counts on FX_EVENT_TURN_END (the turn's totals); context_used and
 * context_size (the model's window) on FX_EVENT_USAGE. Zero when unknown. */
typedef struct fx_usage {
  uint64_t input_tokens;
  uint64_t output_tokens;
  uint64_t cache_read_tokens;
  uint64_t cache_write_tokens;
  uint64_t reasoning_tokens;
  uint64_t context_used;
  uint64_t context_size;
} fx_usage;

/* A typed event. Which fields mean something depends on kind (above); the
 * others are zero. Every fx_str, fx_bytes and names pointer is borrowed until
 * the next fx_events_drain on the same runtime, or fx_runtime_destroy. Hosts
 * ignore kinds they do not know (new kinds are additive). */
typedef struct fx_event {
  fx_event_kind kind;
  uint32_t flags;
  uint64_t turn;     /* the turn id fx_turn_prompt returned, or 0 */
  uint64_t request;  /* the request id the call returned, or 0 */
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

/* WORKER. Moves up to cap queued events, oldest first, into buf and sets
 * *count. Never blocks; *count is 0 when none wait. What earlier events
 * pointed to is released by this call. FX_OK (also on a failed or exited
 * runtime, until FX_EVENT_EXITED has been drained), FX_E_INVALID, or
 * FX_E_CLOSED. Making a text delta costs the core one copy into its event
 * arena and no allocation once the arena has grown; draining costs no copy
 * when cap holds everything queued. */
fx_status fx_events_drain(fx_runtime rt, fx_event *buf, size_t cap, size_t *count);

/* ------------------------------------------------------------------------ */
/* Tool results                                                             */
/* ------------------------------------------------------------------------ */

typedef struct fx_tool_output {
  fx_str text;               /* what the model reads */
  const fx_image *images;    /* frames the model sees, raw bytes */
  size_t image_count;
  bool is_error;             /* text is an agent-oriented error */
} fx_tool_output;

/* ANY. Answers tool call `call` (see fx_host.tool_call). out is copied once,
 * into the core's history. Releases the call's borrowed strings once it
 * returns. FX_OK, FX_STALE (the call was already answered, or cancelled and
 * dropped), FX_E_INVALID (unknown call, oversized result: the model then
 * reads a short error instead), FX_E_CLOSED, FX_E_FAILED. */
fx_status fx_tool_result(fx_runtime rt, uint64_t call, const fx_tool_output *out);

/* ------------------------------------------------------------------------ */
/* Model HTTP (the fetch bridge)                                            */
/* ------------------------------------------------------------------------ */

typedef struct fx_header {
  fx_str name;
  fx_str value;
} fx_header;

typedef struct fx_fetch_request {
  uint64_t fetch;
  fx_str method;
  fx_str url;
  const fx_header *headers;
  size_t header_count;
  fx_bytes body;
} fx_fetch_request;

/* WORKER. Takes the model request the core published, if any. On FX_OK *out
 * describes it; its strings and body are borrowed until the next
 * fx_fetch_take on this runtime or fx_runtime_destroy. The host sends it,
 * adding its own credentials, and streams the answer back with the calls
 * below, from any one thread at a time per fetch. FX_EMPTY when none waits. */
fx_status fx_fetch_take(fx_runtime rt, fx_fetch_request *out);

/* ANY. The response status line arrived. retry_after_s is the Retry-After
 * header in whole seconds, or -1. FX_OK, or FX_STALE (the core gave the fetch
 * up: stop sending it). */
fx_status fx_fetch_status(fx_runtime rt, uint64_t fetch, uint16_t status, int32_t retry_after_s);

/* ANY. Appends response bytes (copied). FX_OK, FX_STALE (stop sending),
 * FX_BACKPRESSURE (8 MiB queued and unread: retry the same bytes shortly; the
 * core frees the space as it reads, without a wake). */
fx_status fx_fetch_push(fx_runtime rt, uint64_t fetch, const uint8_t *data, size_t len);

/* ANY. The response ended: ok true for a clean end, false for a transport
 * failure. FX_OK or FX_STALE. */
fx_status fx_fetch_finish(fx_runtime rt, uint64_t fetch, bool ok);

typedef int32_t fx_fetch_disposition;
#define FX_FETCH_RETIRED  0 /* the core gave it up: abort the HTTP request */
#define FX_FETCH_ACTIVE   1 /* the core still reads it */
#define FX_FETCH_CONSUMED 2 /* the core has all it needs: drain the rest quietly */

/* ANY. Where the core stands on fetch, read atomically. Checked by the host
 * after every wake while the HTTP request runs. Returns FX_FETCH_RETIRED for a
 * closed or failed runtime. */
fx_fetch_disposition fx_fetch_disposition_get(fx_runtime rt, uint64_t fetch);

/* ------------------------------------------------------------------------ */
/* Journal acknowledgements                                                 */
/* ------------------------------------------------------------------------ */

/* ANY. The store holds every append up to and including `append` (the host
 * acknowledges in order; acknowledging a later one acknowledges the earlier).
 * Releases a barrier waiting on them. FX_OK, FX_STALE (already acknowledged),
 * FX_E_INVALID (an id the core never sent). */
fx_status fx_journal_ack(fx_runtime rt, uint64_t append);

/* ANY. The store refused `append` (reason copied). fenced says another writer
 * holds the session: the core then ends the turn with an error and appends
 * nothing more. FX_OK, FX_STALE, FX_E_INVALID. */
fx_status fx_journal_fail(fx_runtime rt, uint64_t append, fx_str reason, bool fenced);

/* ------------------------------------------------------------------------ */
/* Hook answers                                                             */
/* ------------------------------------------------------------------------ */

/* ANY. Answers a pre_tool or stop question that returned FX_HOOK_PENDING.
 * text (copied) is the block reason, the rewritten input JSON or the
 * continuation message; empty for ALLOW. FX_OK, FX_STALE (the turn ended or
 * was cancelled first), FX_E_INVALID (unknown question, or an answer kind the
 * question does not take). */
fx_status fx_hook_answer(fx_runtime rt, uint64_t question, fx_hook_answer_kind kind, fx_str text);

#ifdef __cplusplus
}
#endif

#endif /* FX_CORE_H */
