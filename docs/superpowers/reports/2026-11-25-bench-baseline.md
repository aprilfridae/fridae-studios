Run 2026-11-24T02:45:20 on this Mac (arm64, 12 CPUs); engine source hash 8bb4e509475e; load average at start 14.7 / 9.7 / 9.2.

Corpora:

| Scale | Vault notes | Guides | Team | Distractor notes | Estimated chunks | Engine chunks |
|---|---:|---:|---:|---:|---:|---:|
| 1x | 1311 | 100 | 14 | 0 | 20,541 | 23,464 |
| 10x | 1311 | 100 | 14 | 34369 | 205,412 | 208,140 |

### Quality: hit@1 / hit@3 / hit@5

| Method | Scale | paraphrase | exact | old33 | of which taste |
|---|---|---|---|---|---|
| keyword | 1x | 28 / 31 / 38 (of 58) | 12 / 12 / 12 (of 12) | 17 / 23 / 26 (of 33) | 5 / 6 / 7 (of 10) |
| keyword | 10x | 20 / 21 / 21 (of 58) | 10 / 11 / 12 (of 12) | 7 / 17 / 19 (of 33) | 3 / 4 / 4 (of 10) |
| light | 1x | 27 / 36 / 41 (of 58) | 8 / 12 / 12 (of 12) | 15 / 22 / 25 (of 33) | 3 / 4 / 5 (of 10) |
| light | 10x | 24 / 25 / 27 (of 58) | 9 / 10 / 10 (of 12) | 11 / 15 / 16 (of 33) | 3 / 3 / 3 (of 10) |
| heavy | - | skipped | skipped | skipped | skipped |

### Headline: the drop from 1x to 10x (hit@3, and hit@5)

| Method | Set | hit@3 1x | hit@3 10x | change | hit@5 1x | hit@5 10x | change |
|---|---|---:|---:|---:|---:|---:|---:|
| keyword | paraphrase | 53% | 36% | -17 pts | 66% | 36% | -29 pts |
| keyword | exact | 100% | 92% | -8 pts | 100% | 100% | +0 pts |
| keyword | old33 | 70% | 52% | -18 pts | 79% | 58% | -21 pts |
| keyword | taste | 60% | 40% | -20 pts | 70% | 40% | -30 pts |
| light | paraphrase | 62% | 43% | -19 pts | 71% | 47% | -24 pts |
| light | exact | 100% | 83% | -17 pts | 100% | 83% | -17 pts |
| light | old33 | 67% | 45% | -21 pts | 76% | 48% | -27 pts |
| light | taste | 40% | 30% | -10 pts | 50% | 30% | -20 pts |

### Cost

| Method | Scale | Tokens returned per query (mean / p95) | Query p50 / p95 | rbx_vault end to end p50 / p95 | Queries over rbx_vault's 3 s | Full build | Update after one note changes | Peak memory (build / serving) | Store in memory | Index on disk |
|---|---|---|---|---|---|---|---|---|---|---|
| keyword | 1x | 829 / 953 | 0.29 s / 0.32 s | same | - | 2.70 s (index parse) | 2.98 s (next search re-parses all) | - / 214 MB | - | 13 MB |
| keyword | 10x | 778 / 923 | 3.07 s / 3.47 s | same | - | 27 s (index parse) | 34 s (next search re-parses all) | - / 1.34 GB | - | 115 MB |
| light | 1x | 1259 / 1434 | 1.08 s / 1.64 s | 1.33 s / 1.97 s | 0 | 521 s | 0.31 s (embedded 24 chunks; whole store rewritten) | 738 MB / 1.92 GB | 105 MB | 78 MB |
| light | 10x | 1224 / 1445 | 1.42 s / 1.88 s | 4.59 s / 5.47 s | 0 | 4046 s (vector cache: 46928 of 416280 hits) | 4.28 s (embedded 24 chunks; whole store rewritten) | 1.59 GB / 2.60 GB | 710 MB | 590 MB |
| heavy | - | skipped: needs Ollama (dense mxbai-embed-large) and the Qdrant service | | | | | | | | |

Light, other timings: 1x: open the saved index 0.78 s, the per-query refresh with nothing changed 0.02 s, warm engine query p50 1.10 s / p95 1.53 s, model load 0.43 s; 10x: open the saved index 8.21 s, the per-query refresh with nothing changed 0.25 s, warm engine query p50 1.25 s / p95 2.44 s, model load 0.46 s.

Light 10x build read 46928 chunk vectors from the cache; at the measured 95.5 chunks/s embedding rate a cold build would take about 4538 s.

### Straight-line extrapolation to 100x (from the 1x and 10x measurements)

| Method | Measure | 1x | 10x | 100x (extrapolated) |
|---|---|---:|---:|---:|
| keyword | query p50 | 0.29 s | 3.07 s | 31 s |
| keyword | first search after a change | 2.98 s | 34 s | 345 s |
| keyword | peak memory | 214 MB | 1.34 GB | 12.61 GB |
| light | store in memory | 105 MB | 710 MB | 6.61 GB |
| light | index on disk | 78 MB | 590 MB | 5.58 GB |
| light | peak memory while building | 738 MB | 1.59 GB | 10.26 GB |
| light | full build (cold) | 521 s | 4538 s | 44702 s |
| light | update after one note changes | 0.31 s | 4.28 s | 44 s |
| light | query p50 | 1.08 s | 1.42 s | 4.83 s |

Keyword lost at 10x (in the top 3 at 1x, not at 10x): P08 (now: Economy Resets and Seasonal Wipes, devices, lihfolk-tutorial); P10 (now: Combat Combo Systems, sound-design, Cheat sheet - Combat Combo Systems); P11 (now: Field Notes- Release on death and respawn and Common mistakes, lihfolk-sfx, Layering Camera Feedback on the Default Camera); P19 (now: Voxel Props Environments and Modular Tiles, lihfolk-worldbuilding, When not to follow the and other Voxel Modelling Deep Dive notes); P22 (now: Spawn safety and other Gameplay Systems notes, scout, economy-design); P34 (now: unslop, On The design in one paragraph, producer); P43 (now: Case Study - Grow a Garden (summary), roblox-analytics-liveops, coder); P46 (now: The stair-step problem and other 2D Game Development notes, workflow, Animation-03 Authoring scratchpad - Spacing for arcs and smears); P47 (now: Daily Rewards System End to End, studio, Retention and Return Hooks); T05 (now: Getting flat normals out of and other Untextured Stylised Modelling notes (2), unslop/visuals, lead); T08 (now: Session log - Exploits it must resist, and, roblox-trend-game/trend-games-research, Animation-08 Snippets- Server module and Getting up without a pop); E11 (now: 2 Car destroyed or fallen into - notes, roblox-characters-animation, Kill Planes, Map Boundaries and Out-of-Bounds Recovery - open questions); A02 (now: Anti-Exploit and Security scratchpad - Checklist, Anti-Exploit and Security scratchpad - Assets, Anti-Exploit and Security scratchpad - Bank, do not refuse, for); A06 (now: OrderedDataStores and Leaderboards checklist (draft) (2), Thoughts on OrderedDataStores and Leaderboards (2), OrderedDataStores and Leaderboards checklist (draft)); A07 (now: False Positives Lag and Fair Enforcement - Lag and other honest causes, Lag and other honest causes - notes, Sorting and filtering without lag and other Player QoL and Game Tweaks notes); C03 (now: Shared Close Buttons and Headers Across Many Screens, ui-design, ui); C07 (now: Grabs, Carries and Throws - Moving Players Safely, promo, lihfolk-vfx); C11 (now: Polish (judgement) and other Animation-03 Authoring notes, lihfolk-tutorial, 2D Collision Physics and Platformer Movement - short version (3)); C12 (now: Horror and chase - notes, roblox-performance, Case Study - 99 Nights in the Forest - Why these choices plausibly earn clicks).

Light lost at 10x (in the top 3 at 1x, not at 10x): P03 (now: 3D Modeling- Checklist and Modelling for segmented rigs, Procedural Model Kits - Lessons, scout); P05 (now: Key facts (AnchorPoint), AnchorPoint - my take (2), API and Concept Atlas scratchpad - 56 Field Notes (5)); P06 (now: Edge lead is a grip, Free left hand - field notes, Arm Twist and Grip Roll - Lessons from Baked Sword Clips, Arm Twist and Grip Roll - Lessons from Baked Sword Clips revisited (2)); P10 (now: Combat Combo Systems, Combat Combo Systems - short version (4), Combat Combo Systems in brief); P13 (now: Quantity-limited items, Suggested reading order - field notes, Limited-Time Offers - Design and Ethics, Limited-Time Offers - Design and Ethics - Quantity-limited items); P23 (now: Safety Rules for AI Agents in Studio - my take (2), Safety Rules for AI Agents in Studio - my take (3), Safety Rules for AI Agents in Studio); P33 (now: Down, revive and wipe, Rewards and progression - field notes, Session log - The wave director, Down, revive and wipe, Scaling to party size - field notes); P34 (now: Case Study - Jailbreak revisited, On notes (117), Genre Playbook - Open-World City and Crime Roleplay); P37 (now: Shadow flags and canary rollouts, Aim- what you can and - field notes, Aim- what you can and, Checklist - field notes, Detecting Combat Exploits - Reach Rate and Aim); P43 (now: Live Ops Events and Updates - short version, Case Study - Grow a Garden, Quick notes - Live Ops Events and Updates); P44 (now: Saving Correctly - Autosave and BindToClose, Release Process and Rollbacks - my take, Release Process and Rollbacks); T05 (now: Untextured Stylised Modelling Start Here - my take (2), Getting flat normals out of and other Untextured Stylised Modelling notes (2), The notes (Untextured Stylised Modelling Start Here)); E01 (now: Checklist and other Team Collaboration and Project Management notes, Suggested reading order and other Team Collaboration and Project Management notes, Code Review for Luau Teams - my take (2)); E11 (now: Spawn Points Safe Zones and Respawn Flow, Session log - Layer 1- the engine's destroy, Level Design- Stations and prompts- the hidden and Queue pads, portals and teleports); A02 (now: RemoteEvent - open questions, Global Leaderboards End to End - open questions, Session log - What is in it); A03 (now: Official Docs - UI, Cheat sheet - Official Docs - UI, UI Scale HUD Hide and Compact Mode - my take (3)); A05 (now: Working notes - Session Lock, Working notes - Session Lock (3), Session Lock (summary)); C05 (now: Quick notes - Spawning Teams and Rounds, Spawning Teams and Rounds - short version, Spawning Teams and Rounds in brief); C08 (now: A. Bad publish (broken game, wiped (Disaster Recovery for a Roblox Game - Places, Code, Assets, Data and Accounts), Disaster Recovery for a Roblox Game - Places, Code, Assets, Data and Accounts, Disaster Recovery for a Roblox Game - Places, Code, Assets, Data and Accounts - A. Bad publish (broken game, wiped); C12 (now: Thumbnail Genre Patterns - What Top Games Show - my take, Working notes - Thumbnail Genre Patterns - What Top Games Show (4), Working notes - Thumbnail Genre Patterns - What Top Games Show (2)); C13 (now: Core Concepts- The nuance most people miss and Where this shows up, UI and UX scratchpad - Text (2), Motion Sensitivity and Camera Comfort - todo).

