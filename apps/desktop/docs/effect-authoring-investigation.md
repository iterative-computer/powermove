# Effect authoring investigation — 2026-09-13

The saved Typewriter conversation confirms the reported panel substitution. The
evidence points to a capability/mode mismatch, instructions that encourage an
approximation, and a separate effect-validation failure. It does not establish
that PRs #29–31 removed or broke the effect-registration API.

The initial investigation reviewed merge commits `29437593` (#29), `f3996603`
(#30), `6d850b4b` (#31), and checkout `680cd594`. It did not modify application
source or live project/extension data. Implementation followed on user request.

## Observed failure sequence

The saved conversation `thread-5m71vg3`, titled “Typewriter Effect With Gradient
Cursor,” contains:

1. “make a typewriter effect that lets me control the gradient color of the typing
   cursor” → the agent reports adding a Typewriter panel.
2. “an EFFECT, not a panel” → the agent reports a text layer and cursor layer rig.
   This reply is stored as an error, so it does not establish successful application.
3. “needs to be an effect in effects and presets” → the agent writes an extension
   using `api.effects.register` and declares `contributes: ["effects"]`.
4. Powermove reports `effect "typewriter": too many params (36 > 32)`.

The first two runs' saved tool traces contain only Read and StructuredOutput.
The third has a persistent Claude session with Bash/Write/Edit and the extension
staging directory. This strongly supports Editor-mode execution for the first
two turns and autonomous execution for the third. The saved conversation does
not record access mode, and Editor runs disable session persistence, so the
original access selection cannot be proven directly from those records.

## Why a panel or rig can replace the requested effect

In `src/renderer/src/legacy/assistant/spatial.ts`, `sendRequest` routes according
to `S.accessMode`; it does not route based on the capability the request needs.
For Claude/ChatGPT, Editor mode uses `agentPrompt` and `responseSchema`.

That schema supports sections, chrome, interface, scene, workspace, and panels.
It has no extension-authoring result. Claude's Editor tool set is
`Read,Glob,Grep` plus live inspection tools; it does not receive the autonomous
extension staging/API-pack setup (`src/main/claude/adapter.ts` and `runner.ts`).
The `add_effect` scene command instantiates an existing definition; it cannot
define a new effect (`src/renderer/src/legacy/core/editing.ts`, `addEffect`).

Meanwhile, the Editor prompt explicitly says to prefer a useful executable
interpretation over explaining limitations, and extensively teaches generated
panels and script buttons. It does not require preserving the distinction
between an effect definition, a panel that produces animation, and a layer rig.
This combination explains the observed substitutions without requiring a broken
effects API. Autonomous requests also receive panel focus, panel design, and
placement guidance unconditionally; their effect-authoring guidance is much less
explicit. These prompt weaknesses predate the three merges.

## Why the actual effect then failed

`src/renderer/src/kernel/glsl.ts` enforces `MAX_PARAMS = 32` during registration.
The supplied `api.ts` type declares `params: EffectParamDefinition[]`, and the
extension guide omits that maximum. A model following those documents can write
a definition that compiles successfully but fails activation.

Replaying the original 36-parameter Typewriter source recovered from the saved
Claude Write event, in a temporary directory, produced:

```text
Compile report: PASS
Historical parameter count: 36
Actual kernel rejection: effect "typewriter": too many params (36 > 32)
Control with 32 params: registration validation passes (shader rendering not tested)
```

The control only isolates the registration limit; truncating parameters is not
a proposed fix to the shader. The recorded agent validated syntax, parameter
names, and balanced shader delimiters, but did not run the real kernel validator.
A later run increased the parameter count to 38 and claimed validation against
the TypeScript type, which still could not detect this runtime restriction.

Staging validation in `src/main/codex/change-history.ts` checks manifests and
declared file changes. Main-process extension compilation also does not validate
runtime effect definitions. Activation errors are surfaced later in the renderer
by `applyExtensionChanges`; a successful compile is insufficient evidence that
the requested effect exists and works.

## Attribution to PRs #29–31

- #29 expands the public API and ships a complete Media Browser panel sample.
  It places a substantial panel pattern before the shorter effect example.
  Increased panel bias is plausible, but not demonstrated by a controlled model
  comparison. The effect-registration implementation and its parameter limit
  are preserved.
- #30 adds the fork tool and changes the recommended fork workflow. No evidence
  links that to the observed substitution.
- #31 adds fork rebasing and update notices. Its change to the assistant routing
  file adds a rebase entry point; it does not change ordinary request routing.
- Editor routing, approximation instructions, panel guidance, and the parameter
  limit all exist before #29. Blaming or reverting all three PRs is not supported
  by this investigation.

## Recommended corrections

1. Represent extension-authoring needs explicitly. In Editor mode, identify a
   request for a new effect and explain the required capability/mode instead of
   silently returning a panel or layer rig. Do not silently expand authority.
2. Teach both agent paths that an effect request means an Effects & Presets
   definition with normal effect parameters. A panel is an additional surface
   only when requested or needed for a separate workflow.
3. Ship a complete effect sample and document runtime restrictions alongside
   the public types, including the 32-parameter limit.
4. Provide staged validation/activation feedback to the agent before it claims
   completion. Require evidence of registration, application, and rendered output.
5. Add a regression scenario using the exact Typewriter prompt in each supported
   mode, checking the contribution kind and live registration instead of merely
   accepting a syntactically valid result or compiled extension.

## Validation and limits

The focused existing suite passed: 41 tests across kernel GLSL, kernel registries,
agent instructions, and extension compilation. The historical registration
failure was reproduced against the actual compiler and validator. No fresh paid
model run, before/after model comparison, or visual shader test was performed.
The initial investigation was followed by the implementation below.

## Implemented correction

- Reproduced and fixed access-mode leakage: an API/local provider forces Editor
  mode, and switching back to Claude/ChatGPT previously failed to restore the
  user's saved access choice. Explicit Editor restrictions are still preserved.
- Added a structured Editor-mode capability result and a user-operated
  “Continue with Project access” action that retries the original request.
- Added shared effect-authoring instructions to the native and API providers,
  a complete shipped Gradient Tint effect sample, and the runtime limits in
  the public API types and guide.
- Added `validate_effect`, using the same validator as actual registration, and
  exposed registered effects in workspace inspection.
- Load extensions before applying dependent project commands. Failed loads
  retain their commands through automatic repair, and user changes during repair
  trigger revision-based reconciliation before application.
- Verified the shipped sample in an isolated hidden Electron session: real
  registration, 36-param rejection, animated rendered output, Undo, and no panel
  contribution. Model choices themselves are covered by instruction contracts
  and simulated provider responses, not a fresh paid model evaluation.

## Implementation validation

- Full desktop Vitest suite: 2,036 passed, one skipped (256 files). An existing
  GPU placeholder test timed out on the first run, passed in isolation, and
  passed in the final full run without changes to that test or implementation.
- Type checking: Svelte and main-process TypeScript passed with no errors.
- Extension boundary lint passed for all 10 built-in extensions.
- The isolated Electron effect-authoring test passed against a fresh build,
  including real shader rendering, animated parameters and Undo.
- The live app and the user's project/extension data were not restarted or
  changed for testing. The source changes take effect on the next app launch.
