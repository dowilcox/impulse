# Port collisions between projects: plan

Plan, proposed 9 Oct 2026; nothing decided yet. Not user documentation: this folder isn't published to the website (`scripts/build-site.py` only builds `docs/*.md`). It follows proposal 1 of [Parallel tasks](parallel-tasks.md#1-give-each-task-its-own-environment), which gave each task a slot and ports of its own.

## The problem

A task's slot is unique among its own repository's tasks (`.git/impulse/tasks.json`), and its ports are the main checkout's plus slot × `port_offset`. Nothing on the Mac knows which ports another repository's tasks hold, so two projects with the same base ports and the default offset work out the same numbers. Say pulseboard and trailhead both publish Postgres on 5432: pulseboard's first task gets 5532, and so does trailhead's.

New Task already tries to avoid this. It takes the lowest slot whose ports are all free (`TaskRegistryStore.portsAreFree`, `TaskRegistryStore.swift:58`, through `PortProbe.isFree`, `TaskEnvironment.swift:106`), but the probe only sees what is listening at that moment:

| When trailhead's task is created                                                           | What happens                                                                                                                                          |
| ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| pulseboard's slot-1 stack is running                                                       | 5532 is busy, so trailhead's task skips to slot 2 (5632). Fine.                                                                                       |
| pulseboard's stack is stopped (`docker compose stop`, Docker not running, after a restart) | 5532 looks free. trailhead's task takes slot 1 and writes 5532 into its `.env`. Whichever stack starts second fails with "port is already allocated". |
| The port is a fixed one in the Compose file, moved by `compose_override`                   | The probe isn't asked about it at all (see [below](#bugs-in-what-ships-today)). Both tasks get 8100 for `"8000:8000"`, running or not.                |

After that nothing looks again: a task's values are written once, when it's created.

Two related cases come from the same gap:

- **Within one project.** When the main checkout's base ports are an offset apart (`APP_PORT = 8000`, `ADMIN_PORT = 8100`), slot 1's `APP_PORT` is the main checkout's `ADMIN_PORT`. The probe catches it only while the main checkout's stack runs.
- **Two main checkouts on the same base.** pulseboard and trailhead both on 5432 collide in slot 0 already. Impulse never moves a main checkout's ports, so it can only say so.

### Bugs in what ships today

Found while looking into this; [proposal 1](#1-fix-the-free-port-check) fixes them, and none needs the rest of the plan.

- **Compose ports aren't checked.** With `compose_override = true`, a task's override moves the Compose file's fixed host ports by slot × offset (`ComposeFile.override`, `ComposeFile.swift:186`), but `portsAreFree` only checks `[worktrees.ports]` and returns true when that's empty. A project that only uses `compose_override` gets the lowest slot its registry has free, whatever is listening on the ports it moves to. The New Task sheet shows no Values line for such a project either (`TaskWorkflows.swift:351` only picks a slot when `hasTaskValues`).
- **IPv6-only listeners look free.** `PortProbe` binds IPv4 only, on `0.0.0.0` and `127.0.0.1`. A server on `[::1]` alone, where Node and Vite often end up when told `localhost`, blocks neither: tested 9 Oct, a listener on `[::1]` probed free on both addresses. A listener on `[::]` (both families) is caught.
- **`nextSlot` never gives up** (`TaskRegistry.swift:80`). `PortProbe.isFree` is false above 65535, so once every remaining slot's ports pass 65535 the loop doesn't end and task creation hangs on its background queue. With a base of 8000 and an offset of 100 that takes 575 slots; a high base or offset gets there much sooner, and so will slots shared between projects ([proposal 2](#2-one-list-of-claimed-ports-for-the-whole-mac)).
- **Undo of an archive doesn't check ports.** When another task took the archived task's slot meanwhile, `TaskRegistryStore.restore` (`TaskRegistryStore.swift:99`) gives it `registry.nextSlot()` with no port check, and nothing rewrites the task's values for the new slot (`TaskCleanup.restoreArchived`, `TaskCleanup.swift:74`).

## Proposals

Sizes are rough: **S** is a day or two, **M** about a week.

### 1. Fix the free-port check

**Size S.** Fixes: the four bugs above.

- **One answer to "which ports does slot n use".** `[worktrees.ports]` moved by slot × offset, plus, with `compose_override`, the Compose file's fixed host ports moved the same way (`ComposeFile.Port.host`, already parsed). Env ports are named by their key (`DB_PORT`), Compose ports by service and port (`postgres:5432`). The probe, the New Task sheet's Values line and the claimed-ports list ([2](#2-one-list-of-claimed-ports-for-the-whole-mac)) all use it. The Compose file read is the task's own once its folder exists, and the main checkout's for the sheet's preview.
- **The sheet shows Compose ports.** A project that only uses `compose_override` gets a Values line too: "postgres 5532 · app 8100".
- **IPv6.** `PortProbe.isFree` also binds `AF_INET6` on `::` and `::1`, with `IPV6_V6ONLY` so each bind tests one family. A Mac where an IPv6 socket can't be made at all counts as free for IPv6, not busy, or every port would look taken.
- **A limit.** `nextSlot` takes the highest slot worth trying, and returns nil past it. The caller works it out from the ports: the last slot whose highest port is at most 65535. Creating the task then fails like any other creation failure (`model.error`): "No slot has free ports: API_PORT would pass 65535. Lower `port_offset` or archive a task."
- **Undo of an archive.** A restored task whose slot was taken gets one through the same check as a new task, and its values and Compose override are rewritten for it (`writeTaskValues`).

**Where.** The slot's ports in `TaskEnvironment` (ImpulseKit), taking the parsed `ComposeFile`. `PortProbe` stays where it is. `TaskRegistry.nextSlot` gains its limit. Tests: Compose ports in a slot's ports, the limit, an IPv6-only listener seen as busy (a real socket on `[::1]`, as the existing `PortProbe` test does for IPv4).

### 2. One list of claimed ports for the whole Mac

**Size M.** Fixes: two projects' tasks getting the same ports while one of them is stopped; the within-one-project case.

**Problem.** Each repository's registry only knows its own slots, and the probe only knows what is running. What's missing is a record, across repositories, of which ports Impulse has handed out.

**Proposal.** Impulse keeps one list of the ports it has given out on this Mac, and a new task's slot has to avoid them as well as anything listening.

- **The file.** `~/Library/Application Support/impulse/ports.json`, at that fixed path rather than through `AppPaths`: Impulse Dev keeps its own state in `impulse-dev`, but ports belong to the Mac, so both builds read and write the same list. It's changed under a `flock`, as `TaskRegistry.update` does. Creating a task takes the registry's lock first, then the list's, always in that order.
- **What's in it.** One entry per claimed port: the port, its name (`DB_PORT`, `postgres:5432`), the repository (its shared git folder), the checkout (the task's folder, or the main checkout) and the slot. Entries hold ports, not slots, because projects have different bases and offsets: pulseboard's slot 1 can be trailhead's slot 3, or its main checkout.
- **Claimed when.**
  - A task is created: its ports, in the same locked step that gives it its slot (`TaskRegistryStore.recordCreated`).
  - **Re-apply Values** runs: each task's entries are replaced by the ports just written.
  - A main checkout's project settings are loaded with ports in them (its workspace opens, Project Setup saves): its base ports, as slot 0, replacing what it claimed before. Settings that no longer name a port release it.
  - An archived task comes back through Undo.
- **Released when.** A task is archived (`TaskCleanup.swift:55`), or moves to another slot ([3](#3-show-collisions-and-move-a-task-out-of-one)).
- **Choosing a slot.** The check `recordCreated` and the sheet pass becomes: every port of the slot is free on the Mac and not claimed by any other checkout. With identical settings, pulseboard's tasks take slots 1, 2 and 3, and trailhead's then take 4, 5 and so on. A port claimed by the main checkout of the same repository counts too, which covers the within-one-project case.
- **Entries left behind.** Each read drops entries whose checkout folder no longer exists (a task removed by hand, a repository deleted), the rule `TaskRegistry.reconcile` already uses. A folder on a volume that isn't mounted is kept, since it will come back. The worst a stale entry can do is make a task skip a slot.
- **What it doesn't cover.** Anything Impulse didn't set up: a Homebrew Postgres, another tool's containers. Those are still only seen by the probe, while they run.

**Where.** `PortLedger` in ImpulseKit, beside `TaskRegistry` (Foundation-only: load, locked update, claim, release, prune, collisions), with tests. Wired in `TaskRegistryStore` (`recordCreated`, `remove`, `restore`, and a `portsAreAvailable` that replaces `portsAreFree`), in `reapplyTaskValues` (`MainWindowController+ProjectSetup.swift:284`), and where a workspace's project settings are loaded.

### 3. Show collisions, and move a task out of one

**Size S to M.** Fixes: collisions made before the list existed, and the ones it can't prevent.

**Problem.** The list keeps new tasks apart, but existing tasks were given slots without it, two main checkouts can share a base, and **Re-apply Values** after changing the base ports can land one task on another's ports.

**Proposal.**

- **Filling the list for existing tasks.** When a repository's tasks are first listed (the sidebar's first `OverlapMonitor` pass for it), each task with a slot claims the ports its settings give it. A port another checkout already holds is recorded as a collision, not refused.
- **Where it shows.** A chip on each affected workspace row, beside the overlap chip (`WorkspacesSection.swift:318`), with the other side named: "Port 5532 is also trailhead's fix-login". Both rows get it. Project Setup's Ports section lists the repository's collisions.
- **Move to a Free Slot.** In the chip's popover, for a task, never a main checkout. The task takes the lowest slot that's free by the rules in [2](#2-one-list-of-claimed-ports-for-the-whole-mac), its old claims are released and the new ports claimed, and its values and Compose override are rewritten (`writeTaskValues`, `TaskWorkflows.swift:746`). Which task moves is up to you: it's the row you click.
- **After a move.** Running containers keep the old ports until they're recreated, so the toast says which values changed and offers **Restart Stack**, which runs `docker compose up -d` in a new terminal in the task when it has a Compose file. Impulse never restarts a stack on its own. Values built from `{slot}` change too. Terminals already open keep the old `IMPULSE_TASK_SLOT`; new ones get the new one.
- **Main checkouts.** Two main checkouts on the same port get the chip and nothing to click: the popover suggests changing the base port in one project's settings.
- **No notification.** Collisions appear when the list is filled in or values are re-applied, both while you're looking, so the chip is enough. Overlapping files notify because agents make them while you're elsewhere; ports don't move on their own.

**Where.** Collisions from `PortLedger`, with tests; the backfill next to `OverlapMonitor`'s first pass; the chip and popover in `WorkspacesSection.swift`; moving the slot in `TaskRegistryStore` beside `restore`.

## Order

| Phase | #   | Proposal                             | Size   | Needs          |
| ----- | --- | ------------------------------------ | ------ | -------------- |
| 1     | 1   | Fix the free-port check              | S      |                |
| 2     | 2   | One list of claimed ports            | M      | 1's slot ports |
| 3     | 3   | Show collisions, Move to a Free Slot | S to M | 2              |

1 ships on its own first: the Compose and IPv6 gaps and the hang affect a single project today, with no second project involved.

## Open questions

Each has a recommendation; none is decided.

1. **Main checkouts in the list.** Claiming their base ports means another project's task avoids them even while that main checkout's stack is down. Recommended: yes.
2. **One list for Impulse and Impulse Dev.** Recommended: yes. Two lists would let the two builds hand out the same ports, which is the bug this plan fixes.
3. **No free slot.** Refuse to create the task (recommended), or create it without values, where it would collide with the main checkout.
4. **Restart Stack.** Offer `docker compose up -d` after a move (recommended), or only say a restart is needed.
5. **Notify on a collision.** Recommended: no, as above.

## Also to do

- **User docs.** `docs/project-config.md`: the **Slots** bullet under [Ports and values for each task](../project-config.md#ports-and-values-for-each-task) (a new task also skips ports another project's tasks hold; Compose ports are checked too) and [Docker Compose in tasks](../project-config.md#docker-compose-in-tasks). `docs/tasks.md`: the collision chip and **Move to a Free Slot** under [Tasks in the sidebar](../tasks.md#tasks-in-the-sidebar), and a line in [Gotchas](../tasks.md#gotchas) about tasks created before the list.
- **CLAUDE.md.** `PortLedger` in ImpulseKit's list in the architecture tree, and in the Project config key pattern beside `TaskEnvironment`. `AGENTS.md` doesn't name ImpulseKit's types, so it stays as it is.
- **CHANGELOG.md.** The fixes in 1 under Fixed; 2 and 3 under the release they ship in.
