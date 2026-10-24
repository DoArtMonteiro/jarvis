# 0018. The Windows worker cannot make win32k system calls (amends 0013)

**Status:** Accepted

## Context

ADR 0013 left `WIN32K_SYSTEM_CALL_DISABLE` unset because it had not been validated with CPython. The worker's access to the screen and keyboard state is a known gap: it can obtain a screen device context, and whether it can read pixels or key state is not proven either way. The job's UI restrictions do not apply when the Core itself runs inside a job (CI, some terminals), because Windows does not allow them in a nested job. A private window station is not available to a standard user (ADR 0017).

The win32k lockdown is a per-process mitigation set at creation. Its documented effect is that the kernel refuses the process every win32k system call (window, GDI and input). The Core checks that the kernel reports the policy as set; no test exercises a particular call. It does not depend on the job, so it holds whether or not the Core runs inside one.

## Decision

The worker starts with `PROCESS_CREATION_MITIGATION_POLICY_WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON`. Before the suspended worker runs, the Core reads the policy back with `GetProcessMitigationPolicy(ProcessSystemCallDisablePolicy)` and kills the worker if it is not in force, alongside the existing token and job checks.

Under the lockdown, Python's `ctypes` fails to initialise. The worker never imports it. Containment probes that need Windows calls use the interpreter's built-in `_winapi` module, or read the worker's token and mitigation policy from the test process, as the kernel reports them.

## Evidence

On Windows 11 (build 26300) under a standard account, with CPython 3.12 and 3.14, both with the Core inside a job (no UI restrictions) and outside one (UI restrictions applied), every Rust test suite passed. That covers worker start-up, the stdio protocol, approvals, restarts, crash recovery, shutdown, the start-up isolation probe, the gaming governor and the containment probes.

Two tests pin the new properties:
- `the_worker_cannot_make_win32k_system_calls` reads the policy from outside, with this process as the control.
- `the_worker_cannot_load_a_foreign_function_interface` checks that the import fails confined and succeeds unconfined.

## Consequences

- The worker can no longer reach `user32` and `gdi32` functionality, and Python code in it has no foreign-function interface. A future worker dependency that needs either cannot run there. This is intended.
- This removes one property. It does **not** by itself prove the screen, clipboard, keyboard-state and window-object channels closed: no test reads pixels, key state or the clipboard, and none is planned that would touch real user content. Those rows in the threat model stay open until a probe with synthetic content shows each one refused.
- The registry gap is unaffected. A less privileged AppContainer (LPAC) remains the planned control. With the existing grants, CPython does not start under it: process initialisation fails with access denied.
- The mitigation list in the isolation report names it ("no win32k system calls").
