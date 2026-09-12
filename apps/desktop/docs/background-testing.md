# Agent testing without visible windows

Verify work against the running Powermove instance through the `powermove` tools, not by launching another copy of the app. An Agent Workspace contains the API pack, the project snapshot, and staged extensions; it does not contain Powermove's source, package scripts, or test harness, and none of those are available to an agent.

For extensions, the staged folder is compiled and validated when the run's result lists it in `extensions`. Powermove reloads it in the live editor, and the run report shows compile errors. Use `get_panel_layout`, `open_panel`, `get_panel_state`, `interact_panel`, `capture_panel`, and `computer_use_panel` to exercise a panel and take real screenshots. Use `get_workspace_state` for layout, selection, and recent errors. Use `render_frames` to review composition output.

Never open a new window for testing. Never launch a headed browser, DevTools, or a second visible copy of Powermove, and never point anything at the user's live profile. If a check cannot be performed with the tools above, report it as unverified instead of building a substitute harness.

Report exactly what was tested. Panel screenshots and interactions prove renderer behavior, not native focus, file dialogs, permission prompts, or OS behavior. Those require explicit user coordination.

Thread controls save separate conversations, drafts, attachments, and focus choices per project. Titles come from the first user message. A thread keeps its own Codex session for each authority. Finish or stop a run before switching. Pending edits/checkpoints are retained during a switch only while the document revision is unchanged, and are not restored across app launches.

Component and extension hot updates are the preferred path for the live editor. When a core engine change has no safe hot replacement, preserve the current editing session, report the pending update, and ask before restarting. Do not mistake a successful compile for a live update.
