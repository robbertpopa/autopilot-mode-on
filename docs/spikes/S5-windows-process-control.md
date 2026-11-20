# S5: start `claude.exe` with no window and kill its whole tree on Windows

**Question:** how do we run a headless agent with no console window, read its output, and guarantee no orphan processes after cancel, timeout or a crash of our own program? **Gates:** live execution in M1. **Spending:** none (use `tools/fake-agent` or any console program, plus one real `claude --version`).
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S5-windows-process-control.md`."

## Method: a small Rust program (later becomes the shared `process` supervisor)
- [ ] Prototype a suspended Windows launch with `CREATE_NO_WINDOW`, then attach it to the job before resuming. Pipe stdin for the prompt and drain stdout and stderr concurrently with bounded buffers.
- [ ] Put the child in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` before it can launch descendants. Verify startup aborts if assignment fails. Keep `unsafe` in one small module with `// SAFETY:` comments.
- [ ] Find `claude.exe` (`which`, then `%USERPROFILE%\.local\bin\claude.exe`); handle a `.cmd` shim by spawning the real binary.
- [ ] Test cases: normal exit; cancel while running; timeout; the child spawns a grandchild that sleeps (is it killed?); our program is killed with Task Manager (does the job kill the children?); a very large output; a child that ignores stdin closing.
- [ ] Confirm no console window appears (run from a GUI parent and watch) and that CPU stays idle while waiting.
- [ ] Test Linux process-group cancellation and separately verify a supervisor lifetime mechanism for abrupt parent exit. Enable unattended execution only after that target passes both checks.

## Done when
All test cases pass on Windows and Linux; the runner API and its pitfalls are written into `docs/pitfalls.md`; the code is ready to move into `crates/process`.

## Result
(fill in)
