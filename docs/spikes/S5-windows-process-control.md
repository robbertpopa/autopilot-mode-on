# S5: clean up supervised process trees

**Question:** how does the launcher manage the console, pipes and lifetime of a synthetic worker and its descendants? **Gates:** live execution in M1. **Spending:** none. Live provider launch and native executable selection are part of the M1 adapter.
**Start prompt:** "Read `AGENTS.md` and `docs/IMPLEMENTATION-PLAN.md`, then do `docs/spikes/S5-windows-process-control.md`."

## Method
- [x] Launch with `CREATE_NO_WINDOW` and attach the Windows job at creation through a `STARTUPINFOEXW` job list. Pipe stdin and drain stdout and stderr concurrently with bounded buffers.
- [x] Configure a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Supply the job during process creation and surface launch errors. A separate create-and-assign sequence leaves a gap where supervisor death can strand the child. Windows API calls are confined to the platform module.
- [x] Test normal exit, cancel, timeout, forced supervisor exit, output flooding, a worker that ignores stdin closure, and a missing executable. Tests verify the synthetic worker and grandchild stop.
- [x] Verify a synthetic console child started by a Windows GUI parent has no console. Measure supervisor CPU while waiting for a timeout.
- [x] Test Linux process-group cleanup after cancellation and abrupt exit. A sibling guardian observes EOF on the owner's private control pipe, stops the worker group and reaps descendants it adopts.

## Done when
The synthetic cleanup checks pass on native Windows and in a Linux container. The shared runner lives in `crates/process` and its limits are recorded below.

## Result

On 2026-11-20, all five automated lifecycle tests passed on Windows and Linux. The Windows GUI probe measured less than 1 ms of supervisor CPU at its reporting precision over a 2,001 ms timeout. The installed native `claude.exe` reported version 2.1.329. The provider adapter still needs its own launch and authentication smoke test, including native executable selection on installations that expose a `.cmd` shim.

Windows tree cleanup uses the creation-time job list and limits handle inheritance to the child's standard streams. Linux cleanup applies to descendants that stay in the worker's process group. Pipe waits have a cleanup deadline. S6 covers filesystem, credential and network containment. [Windows process attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
