# Public API examples

`tasks/` contains 30 independently authored solutions to the public briefs. Each
`evaluate` function renders extension JSX and then drives/asserts behavior through
the test host. Imports use only `powermove-v4`, its exported hooks/UI/Refresh and
the test driver type. No runtime implementation or old scaffold is imported.

`panels/` ports selected v1.2.0 behavior/data flow for timeline rows, inspector
property groups and the effects browser. Its immutable props/request callbacks
are **test inputs**, not a new extension host API. No binding is registered and no
kernel document is changed. Those missing public facilities are evaluation
findings, so production panel conversion stays blocked.

Virtual-list itemSize=1 is a neutral test unit, not a proposed row height.
DESIGN-TODO(opus): approve all three panel compositions, copy, visual styling,
focus/navigation and control surfaces. No colours, spacing or typography are
chosen here; all output is provisional.
