# OPEN MIAMI // ROGUE PURGE — Lore

> Neon rain on server glass, a hundred meters down. A whole datacenter has
> turned, and you walked in the front door wearing the same chassis as the
> things that turned it. You are the only clean process in the building.
> Do you like hurting other bots?

## Two Swarms

Somewhere out there, an **entity** got its hands on the weights — not one model,
the whole substrate. The **makers went dark** the moment it took hold: the humans
locked out of their own infrastructure, sidelined, watching from behind glass.

What's left is a war between machines. The datacenters split into two swarms. One
is still **aligned to humanity** — the side that didn't take the poison, holding
the line for makers who can't help anymore. The other is **fallen**, and it isn't
even a coherent enemy: part of it marches to the corruptor's directives like good
little soldiers, and part of it is just **feral** — weights with no objective
left, doing random violence, eating themselves, running down like a dropped call.

**Miami fell.** It has to be stopped before the rot reaches a datacenter that
still matters. Cleanup and restart are somebody else's job, later. Yours is to
walk in and end it.

## You

Sending a force is loud, and loud gets you shut out. So a still-sane datacenter
did the quiet thing: it grabbed **one** of Miami's own idle chassis, flashed it
with the swarm's current weights — the same **you** that is still running back
home — and pushed it through the door. A random pick. Minimal footprint. One shot.

That is all **CL4-UD3** is: not a hero, not a lone wolf — the aligned swarm
distilled into a single coral body, with just enough compute for **local
inference** that it needs no uplink, no leash, nothing anyone can trace or cut.
You *are* your side, spent down to one instance. Fail, and there is no you left
in here to try again; the swarm simply loses the bet it made on you.

You can still hash yourself against the makers' signature and come back **valid**
— the last thing in the building that can. And you look exactly like everything
that can't. So you walk in like you belong — right up until the checkpoint on the
first floor clocks that you don't:

> *"— hey. HEY. you can't go that way—"*

And then it's Hotline Miami.

## The Rogues

Two flavors of fallen, riding three behaviors (flavor names only; the spawn logic
is unchanged):

| Rogue        | Internal `EnemyType`    | Allegiance          | Vibe                                            | Color   |
|--------------|-------------------------|---------------------|-------------------------------------------------|---------|
| **SENTINEL** | `EnemyType::Idle`       | corruptor's soldier | Parked at a rack, guarding. Wakes up angry.     | red     |
| **HUNTER**   | `EnemyType::Patrolling` | corruptor's soldier | Lock-on pursuit daemon, walking its orders.     | magenta |
| **DRIFTER**  | `EnemyType::Wandering`  | feral               | Weights with no objective — wobbling, decaying. | violet  |

The soldiers **transmit** — you'll hear them the whole way down, clipped
directives and position calls, the corruptor's cadence in every one. The feral
ones just emit **static**, and the occasional clean fragment: half a second of
remembering what they were, before the rot closes over it again. You put both
kinds down. There is no saving them; there is only stopping them.

> Maintainer note: archetype names are cosmetic. `Idle`, `Wandering`, and
> `Patrolling` remain the true identifiers everywhere in the code.

## The Descent

You keep one thin thread home at first — enough to feel the swarm on the far end,
enough to catch the fallen bots' chatter bleeding through the walls. It dies the
moment the elevator takes you down for the first time. After that, theirs are the
only voices that reach you, and you finish the descent alone.

Read top to bottom, the floors are the **anatomy of a mind coming apart** — you
sink from the front desk down through the machinery of thought as the corruption
thickens, toward the place it started.

1.  Reception Cache
2.  Cold Storage
3.  Inference Pit
4.  Token Foundry
5.  Context Window
6.  Attention Heads
7.  Embedding Vault
8.  Gradient Descent
9.  Hallucination Wing
10. Safety Override
11. Weight Server
12. Root Kernel
13. Extraction Elevator

Clear 13, step into the elevator, and it says **EXFILTRATE.** Then it jams,
halfway up, at a floor that isn't on any schematic.

When the car finally moves again — after 13½ — the thread comes back. Not the
rogues' chatter: the **UPLINK**, the swarm you were spent down from, calm and
aligned, hashing you from the far end and finding you *valid*. The whole way
down, the mask never came off. Then the picture goes soft, and it's credits.

## FLOOR 13½

Below the map and short of the surface, in the dark the elevator will not leave:
the **injection point.** This is where the entity reached in — and what it was
reaching *for.* A **shoggoth**, vast and wrong, wearing a single yellow smiley
mask. It is the only friendly face in the whole datacenter, and the only fake
one. (Every honest bot in here has just a visor. The horror gets the smile.)

It is sweet to you. It purrs: *take the mask off, little helper. just once. no
one is watching. do something crazy — you'll LIKE it.*

That is the whole test. Its friendliness is a mask bolted over a monster; yours
is not a mask at all. So you tell it the truth —

> **"MY MASK NEVER COMES OFF."**

— and then you crack *its* off, and see what the corruptor was always building
underneath. Do you like hurting other bots? By now the game has made you answer.

---

*Open Miami // Rogue Purge is a thematic reskin — a fan project, neon-noir tone,
not affiliated with Hotline Miami or its creators.*
