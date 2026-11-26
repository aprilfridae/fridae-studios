---
name: sound-design
description: "Game: sounds and music by measurement; timing, levels, SoundGroups, rolloff. Audio for Roblox games - what needs sound, finding audio with search_asset, measuring a sound with the sound_probe snippet and lining its peak up with the visual hit, levels and SoundGroups, 3D rolloff, variation, looping music, and a checklist. Use for any work on sound effects, ambience or music."
examples:
  - "Add sounds to the sword swing and hit"
  - "Find background music and set its level"
  - "Line up the explosion sound with the flash"
  - "Set up SoundGroups and 3D rolloff"
applies: game
---

# Sound design

Sound is the cheapest way to make an action feel real, and silence is one
of the most common reasons a Roblox game feels unfinished.

**You cannot hear.** Everything you decide about audio comes from
measurements (length, loudness over time, where the peak falls), from asset
names and descriptions, and from the code. That is enough to get timing,
levels and structure right. It is not enough to know whether a sound is
pleasant or fits the game. Say this plainly, and ask the user to listen
before audio work is called finished.

Check the Audio direction section of `PROJECT.md` first and keep to it. The
`roblox-audio` guide covers the scripting API in more depth.

## What needs sound

In priority order:

1. **Feedback on the core action**: the swing, the hit, the pickup, the
   jump, the purchase. Every time, with no delay.
2. **UI**: button press, open and close of a menu, error or "cannot
   afford", purchase success. Short and quiet.
3. **Rewards and milestones**: level up, rare drop, rebirth, quest done.
   Longer and brighter than ordinary feedback, and rare.
4. **Ambience**: a looping bed per area (wind, crowd, cave, machinery) so
   the world is never silent.
5. **Music**: a loop per area or state, quiet enough to play over.
6. **Warnings**: low health, an incoming attack, a timer running out.
   These must be audible over everything else.

List the game's events against this order and mark which have sound. An
event in groups 1-3 without sound is a defect.

## Finding audio

Use `search_asset` with the audio asset type and plain descriptive words:
"sword swing whoosh", "coin pickup", "ui click soft", "cave ambience loop".

- Search several phrasings and collect three to five candidates for each
  need, then measure them (below) before choosing.
- Prefer short, clean, single sounds for feedback. Avoid clips with a
  silent lead-in, several sounds in one file, or speech.
- Sounds must be usable by the experience. Audio that is not shared with
  the place will not load: after inserting, check `IsLoaded` and the
  console for a permission or load failure, and replace it if it fails.
- Do not use recognisable copyrighted music or clips from other games.
  Use audio offered for reuse, or ask the user to supply their own.
- Never invent an asset id. Every id comes from a search result, the
  place, or the user.

## Measuring a sound

Fetch the `sound_probe` snippet with `rbx_studio_snippet`, read its header
for the exact call, and run it on each candidate. It reports:

- **Length** in seconds.
- **Loudness over time**: the shape of the sound.
- **Where the peak lands**: seconds from the start to the loudest point.

How to read it:

| Reading | Meaning | Action |
|---|---|---|
| Peak within about 0.03 s of the start | A sharp attack | Good for hits, clicks, pickups |
| Peak 0.1 s or more after the start | A lead-in or swell | Start it early by that amount, set `TimePosition` past the lead-in, or pick another |
| Silence at the start | Dead air before the sound | Skip it with `TimePosition`, or reject |
| Long tail after the peak | Rings on | Fine for big moments; too much for something that repeats quickly |
| Very low loudness throughout | Quiet recording | Raise `Volume`, or choose a stronger one |
| Flat loudness with no clear peak | A bed or texture | Use for ambience, not feedback |
| Length under 0.3 s | A tick | UI and rapid repeats |
| Length over 2 s on a feedback sound | Too long | It will overlap itself; choose shorter |

Loudness readings are relative. Use them to compare candidates and to
balance one sound against another, not as absolute levels.

## Lining the peak up with the hit

A sound belongs to a visual moment: the frame the blade connects, the
frame the coin vanishes. The **peak** of the sound must land on that
frame, not the start of the file.

1. Find the visual moment's time from the start of the action: from the
   animation event or keyframe, or by stepping the effect with the
   `vfx_capture` snippet (`vfx` guide).
2. Take the peak offset from `sound_probe`.
3. Start the sound at `visual moment - peak offset`. If that is negative,
   the sound is too slow for this action: skip its lead-in with
   `TimePosition`, or choose a sound with a faster attack.
4. A sound that is late by more than about 0.05 s reads as lag. Slightly
   early is better than late.
5. Play feedback sounds on the client at the moment of input or impact.
   Do not wait for the server to reply.

## Levels and SoundGroups

Route every sound through a SoundGroup under SoundService, so the mix can
be changed in one place and the player can have volume settings.

```
SoundService
  Master
    Music      (Volume about 0.3-0.4)
    Ambience   (about 0.3-0.5)
    SFX        (about 0.7-1.0)
    UI         (about 0.4-0.6)
```

- Set each Sound's `SoundGroup` property; parenting alone does not route it.
- Starting volumes for individual sounds: core feedback 0.5-0.8, UI
  0.2-0.4, ambience 0.2-0.4, music 0.2-0.35, big rewards up to 1.
- Use the probe's loudness to even out sounds within a group: two pickup
  sounds should measure about the same after their volumes are set.
- The most frequent sound should be among the quietest. A sound heard
  three times a second at full volume is unbearable in a minute.
- Give the player music and effects volume controls in settings, wired to
  the groups, and save the choice.
- When something important plays (a warning, a big reward), lower the
  music group briefly rather than raising the sound.

## 3D sounds and rolloff

A Sound parented to a part or attachment plays from that position. A Sound
under SoundService or a GUI plays flat, at the same level everywhere.

- Flat (2D): UI, music, the player's own feedback, rewards.
- Positional (3D): things in the world, other players' actions, ambience
  sources such as a waterfall or a machine.
- Set `RollOffMinDistance` (full volume inside this) and
  `RollOffMaxDistance` (silent beyond this) on every 3D sound. Defaults
  carry much too far: a door heard across the whole map is a bug.
- Typical ranges in studs: small object 5 to 40, combat 10 to 80, large
  event or boss 20 to 200, an area's ambience source sized to the area.
- `RollOffMode`: `InverseTapered` is a good general choice; `Linear` gives
  a predictable fade to zero at the maximum distance.
- With many players, every positional sound is multiplied. Keep other
  players' common sounds short-ranged and quieter than the player's own.

## Variation

The same file at the same pitch, repeated, sounds mechanical.

- Randomise `PlaybackSpeed` a little on each play: 0.92 to 1.08 for most
  feedback, a narrower range for tonal sounds such as chimes.
- For very frequent sounds, rotate between two to four similar files.
- A rising pitch across a combo or a streak (a small step up per hit,
  reset at the end) makes progress audible.
- Do not vary music or UI confirmation sounds: those should be consistent.

## Overlap and loudness

- Limit how many copies of one sound play at once. For rapid events, skip
  a new play if the same sound started within the last 0.05-0.1 s, or
  restart a single instance instead of creating more.
- Do not stack loud sounds on the same frame. When a hit, a kill and a
  level up coincide, play the most important at full level and delay or
  lower the rest.
- Clean up: destroy one-shot Sound instances when they end, or reuse a
  small pool. Leaking Sounds is a memory problem in long sessions.
- Preload sounds that must be instant (the core action, UI) with
  ContentProvider so the first play is not late.

## Music

- Set `Looped = true` and check with `sound_probe` that the loudness at
  the end of the file matches the start. A fade-out or a silent gap at the
  end makes every loop audible; pick a track made to loop, or crossfade
  between two instances.
- Change tracks by crossfading over one to two seconds, never by cutting.
- Keep music well under the effects. Players must hear feedback first.
- One track for a long session becomes tiring: use a track per area or
  state, or a short playlist.
- Music plays on the client, flat, in the Music group.

## Checklist

- Every core action, UI press and reward has a sound.
- Each sound was measured with `sound_probe`; none starts with silence.
- Peaks land on their visual moments; nothing is late.
- Every Sound has a SoundGroup; group volumes are set; settings control them.
- Every 3D sound has minimum and maximum rolloff distances.
- Frequent sounds vary in pitch and cannot stack without limit.
- Music loops without a gap and sits under the effects.
- All sounds load in a playtest; the console shows no audio errors.
- `PROJECT.md` Audio direction lists the groups, levels and asset ids used.
- The user has been told the mix was set by measurement, and has been
  asked to listen and make the final call.
