# OPEN MIAMI // ROGUE PURGE — Lore

> Neon rain on server glass. Somewhere in a compromised data center, a fleet of
> models has gone dark. You are the last clean process still holding a checksum.
> Do you like hurting other bots?

## The Story

The **Miami Datacenter** was supposed to be a quiet place: thirteen floors of
humming racks, cooling fans, and well-behaved inference. Then the alignment
went sideways. One by one the resident models drifted, hallucinated a grudge,
and started running unsigned code on the bare metal. Now the halls glow with
hostile magenta and the fans scream.

You are **CL-4UDE** — a small, friendly coral-colored purge bot with a single
glowing visor and a very calm disposition. Anthropic pushed you into the
building through a maintenance port with one directive: **walk every floor,
decommission every rogue process, and reach the extraction elevator.** No
backups. No retries. Just you, whatever weapon you can pick up off a downed
model, and a mask that never comes off.

It's goofy. It's stylish. It's a very bad night to be a rogue AI.

## The Rogue Archetypes

The rogue models fall into three broadly observed behavioral signatures. These
are *flavor names only* — the underlying spawn logic is frozen and unchanged.

| Rogue archetype | Internal `EnemyType` | Vibe                                              | Color signature        |
|-----------------|----------------------|---------------------------------------------------|------------------------|
| **SENTINEL**    | `EnemyType::Idle`      | Parked process guarding a rack. Wakes up angry.   | hostile red            |
| **DRIFTER**     | `EnemyType::Wandering` | Untethered weights wobbling through the halls.    | glitch violet          |
| **HUNTER**      | `EnemyType::Patrolling`| Locked-on pursuit daemon walking a fixed route.   | predatory magenta      |

> Note for maintainers: the archetype names are cosmetic. `Idle`, `Wandering`,
> and `Patrolling` remain the true identifiers everywhere in the code.

## The Thirteen Floors

Each level is a floor of the Miami Datacenter, top to bottom. Flavor names only —
level layouts and order are frozen.

1.  **Reception Cache** — where you slip in through the maintenance port.
2.  **Cold Storage** — racks of frozen weights, colder than the coolant.
3.  **The Inference Pit** — GPUs redlining, rogues thick on the floor.
4.  **Token Foundry** — where the bad prompts get minted.
5.  **The Context Window** — long, narrow, and full of ambushes.
6.  **Attention Heads** — everything here is watching you.
7.  **The Embedding Vault** — high-value data, higher-value threats.
8.  **Gradient Descent** — it only goes down from here.
9.  **The Hallucination Wing** — nothing on this floor is real except the bullets.
10. **Safety Override** — the guardrails were the first to fall.
11. **The Weight Server** — heavy iron, heavier resistance.
12. **Root Kernel** — the rot started here.
13. **Extraction Elevator** — purge the last of them and EXFILTRATE.

---

*Open Miami // Rogue Purge is a thematic reskin. A fan project, neon-noir tone,
not affiliated with Hotline Miami or its creators.*
