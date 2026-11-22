# Long tool calls that outlast an AI's cut-off

Approved by the user on 2026-11-22 ("Yes, build it").

## Why

Measured on 2026-11-22: Cursor cuts every call to a tool server off after 60 seconds, on a process per message (how
the installed app runs it) and over ACP alike; Gemini over ACP cuts them off after three minutes. No setting on either
tool changes it. Fridae's own tools often run longer: a Lead waiting on its experts (`rbx_team wait`), a texture or
review waiting for the user, Blender builds, some Studio calls. On Cursor these fail today, and the limit is the only
reason Gemini's kept process is off by default.

## Behaviour

- The capability table gets one number per AI and path: the longest a tool call may run (`tool_call_limit`, seconds;
  none for Claude, OpenCode and Gemini per message unless measured otherwise). The app passes it to its tool server in
  that chat's entry (`RBXKIT_CALL_BUDGET`), with a safety margin taken off (Cursor 60 → 45, Gemini kept 180 → 150).
- With no budget set, every tool behaves exactly as today. Claude's path does not change.
- With a budget, a call that has not finished when the budget runs out returns in time with a plain result: that the
  work is still going, its job name, and the one call that picks it up (`rbx_job` with `wait`), worded so a model does
  not take it for a failure or for the finished result. The work itself carries on in the tool server.
- `rbx_job wait <job>` returns the finished result exactly as the original call would have (text and pictures), or,
  when the budget runs out again, the same "still going" answer. `rbx_job list` shows what is running; `rbx_job cancel`
  ends one.
- Calls that are already waits (`rbx_team wait`, waiting for the user) simply return within the budget saying nothing
  has changed yet and to call again; they need no job.
- Work that must not overlap (Blender, Studio) still runs one at a time: a second such call while one is going answers
  that it is queued behind the named job, within the budget.
- Stop, the end of a turn, closing the chat and quitting end the chat's running jobs with everything they started;
  none outlives its tool server. A turn that ends while a job is still running says so in the chat in one line, since
  the work was thrown away.
- The Lead's and experts' standing text says, in a line, that a "still going" answer means call `rbx_job wait` again,
  not report back.

## Then

- With this in, measure a four-minute Blender-like job on Cursor per message and on Gemini's kept process. If Gemini's
  passes, with a thumbnail seen live and a Full auto write to a protected file refused live (the two things its review
  left open), Gemini comes off `KEPT_ON_REQUEST`. Cursor's kept path stays on request (its sandbox does not apply
  there).

## Non-goals

- Changing any tool's own behaviour or result.
- Jobs that survive the turn or the app.

## Tests

- No budget: results byte-for-byte as before for every tool (the existing Python tests, unchanged, pass).
- With a budget: a slow tool returns in time with the "still going" answer; `rbx_job wait` returns the same result the
  tool would have, pictures included; two waits in a row; cancel; a queued second Blender-class call; a failing job
  reports its error once; the tool server ending takes its jobs with it.
- The capability number reaches the tool server's entry for each AI and path; Claude's entry has none.
- One real turn each on Cursor per message and Gemini kept, recorded as fixtures: a job longer than the cut-off
  finishes and the model reports its real result.
