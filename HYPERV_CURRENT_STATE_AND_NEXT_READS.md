# Hyper-V Current State And Next Reads

## Summary

The earlier analysis about a "fresh bootstrap run" is not the same as the last manually stabilized VM state.

What we actually have right now is:

- The VM can exist and run with the manually stabilized VHDX path.
- The worker can be healthy and reachable at `192.168.250.10:3031` when that manually stabilized state is preserved.
- The major remaining problem is not "the worker model is impossible." The problem is that a fully fresh automated bootstrap path is still not reproducing that good state consistently.

## Important Correction

The prior analysis described the state of a fresh bootstrap that:

- removes VM artifacts,
- recreates the VM from the scripted image path,
- depends on guest bootstrap behavior to re-establish the worker automatically.

That is not identical to the last known-good manual state.

The real distinction is:

- **Manual stabilized state:** workable enough to reach a running VM worker.
- **Fresh automated bootstrap state:** still unreliable, especially when the VM is destroyed and recreated from scratch.

## Current Real State

When the manually stabilized state is preserved:

- The VM is running.
- The worker can be reachable at `192.168.250.10:3031`.
- The system is usable enough that the main risk is destroying that state and failing to reproduce it automatically.

What breaks the flow is:

- `Remove-VmArtifacts`
- a fully fresh bootstrap path
- guest bootstrap steps that do not consistently restore the worker/network state on their own

## Real Problem Statement

The core problem is:

The scripted fresh bootstrap path still does not reliably recreate the known-good manual VM worker state after the VM is torn down.

More concretely, after a destructive reset/rebootstrap:

- the VM may boot,
- but guest initialization may not re-establish the worker the same way the manual repaired state did,
- so the automation path is still weaker than the manually recovered state.

## Working Hypothesis

The likely failure zone is guest-side post-boot setup, not the high-level coordinator design.

The bootstrap path may successfully:

- create the VM,
- attach disks,
- seed cloud-init,
- boot the guest,

but still fail to fully reproduce:

- network readiness,
- worker service readiness,
- token alignment,
- or post-boot worker installation/startup.

## Read-Only Plan Of Action

These are the next files and surfaces to inspect without changing behavior first:

1. `scripts/hyperv-worker.ps1`
   - Focus on `Get-CloudInitUserData`
   - Focus on `Start-WorkerVM`
   - Focus on `Wait-WorkerHealth`
   - Goal:
     - determine whether a post-boot SSH provisioning step should run after boot and before health timeout
     - verify exactly what the script assumes cloud-init will complete automatically

2. `src/worker.rs`
   - Verify:
     - auth header name
     - worker URL usage
     - token flow from config/env into requests
   - Goal:
     - confirm the coordinator is speaking the contract the worker actually expects

3. `desktop/hyperv.js`
   - Verify:
     - how the worker token is generated
     - how it is persisted
     - how it is passed into bootstrap
   - Goal:
     - confirm the desktop launcher and worker are using the same persisted token source

4. `desktop/main.js`
   - Verify:
     - when UAC elevation is triggered
     - whether Hyper-V bootstrap runs every launch or only in specific missing/broken states
   - Goal:
     - separate first-time provisioning from normal daily launch behavior

## What Not To Confuse

Do not conflate these two states:

1. **Known-good-enough manual VM state**
2. **Fully fresh, fully automated rebuild-from-zero state**

The repo still needs work to make state 2 reproduce state 1 reliably.

## Practical Conclusion

The correct short-term engineering goal is not to redesign the entire coordinator/worker model again.

It is to make the automated bootstrap path recreate the already-proven manual VM worker state consistently.

That means the likely next real fix is in bootstrap orchestration and guest post-boot provisioning, not in the top-level architecture.
