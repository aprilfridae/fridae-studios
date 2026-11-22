# VFX basics

Working notes for building Roblox effects with the rbxkit Emit module. A
toolbox, not a style: depart from any of it when the effect calls for it, and
judge by captures, not by numbers.

## What an effect is for

Decide this first, because it sets size, duration and brightness.

- **Feedback** (a hit landed, a pickup collected): small, fast, over in
  0.2-0.5 s, never hides the target.
- **Telegraph** (something is about to happen here): readable shape on the
  ground or in the air, holds long enough to react to, calm colours until the
  moment.
- **Payoff** (an ultimate, an explosion): fills a real share of the screen
  for a moment, then clears quickly so play can continue.
- **Ambient** (aura, idle glow): low contrast, loops, never competes with
  gameplay effects.

Work out how big it is **from the player's camera**, not close up. A hit seen
from 30 studs needs shapes several studs across to read at all.

## Where effects live

- One Model per effect in `ReplicatedStorage.VFX`, named after it.
- An invisible root part (`Transparency = 1`, anchored, no collision), with
  Attachments holding emitters. One emitter per layer, named for the layer.
- Play with `Emit.play(name, at)`; it clones, places, plays and removes.
  `Emit.emit(instance)` plays emitters already on a weapon or character.

## Attributes the player reads

| On | Attribute | Meaning |
|---|---|---|
| ParticleEmitter | `EmitCount` | Burst this many (max 500) |
| ParticleEmitter | `EmitDuration` | Emit at `Rate` for this long |
| Beam, Trail | `EmitDuration` | Enabled for this long |
| Light | `EmitDuration` | On, then fades to 0 over this long |
| any of those | `EmitDelay` | Seconds after the effect starts (absolute) |
| Model/Folder with `Start` and `End` parts | `Duration`, `Easing`, `EasingDirection`, `Spin`, `EmitDelay` | `Start` tweens to `End`'s CFrame, Size, Transparency, Color |
| effect root | `EffectDuration` | When the effect ends |

A burst emitter has `Rate = 0` and an `EmitCount`. An emitter with neither
attribute streams forever and ignores slow motion: give it `EmitDuration`.

## Layers

Most effects that read well are 4-8 layers, each doing one job:

1. **Flash**: one bright shape at the first frame, larger than what follows,
   gone within about 0.1 s. `LightEmission = 1`, few particles, big size.
2. **Core / body**: the shape that carries the effect for its middle
   (crescent, flame tongues, shock ring). Drawn, hard-edged, coloured.
3. **Sparks / debris**: small, fast, many, with `Drag` so they slow, and
   `Acceleration` down for weight. Stretch them along velocity
   (`Orientation = VelocityParallel`, `Squash`).
4. **Smoke / dust**: slow, larger, darker, `LightEmission = 0`, outlives the
   rest and fades. This is the dark value that makes the bright layers read.
5. **Glow**: a soft round sprite behind everything, low alpha. One layer, not
   the effect.
6. **Light**: a PointLight with `EmitDuration` so the world reacts.
7. **Ground / world**: a ring mesh that grows (Start/End pair), a scorch
   decal, a crack texture.

Stagger them with `EmitDelay` instead of firing everything at t = 0.

## Timing

Effects have a curve: anticipation, hit, body, settle.

- Anticipation (optional, 0.05-0.3 s): something gathers or shrinks inward.
- Hit: the flash and the fastest sparks, in the first 2-4 frames.
- Body: the main shape, peaking around 0.1-0.2 s and readable until 0.3-0.5 s.
- Settle: smoke and embers fade; nothing bright after about 0.6 s for a hit.

Fast out, slow settle. Use `Size` and `Transparency` NumberSequences that
jump up early and ease out, not straight ramps. Ease-out (`Quad`/`Quint` Out)
for anything that bursts.

## Colour, value, brightness

- Pick about five colours: a near-white hot core, the main hue, a darker and
  more saturated version of it, an accent, and a dark (smoke, shadow).
- Colour over life goes hot to cool: white, then the hue, then the dark hue.
- Without a dark layer the bright ones read as a white blob. On a pale floor
  the darks matter most.
- Stacked additive layers add up to white. If the centre is a blob, cut
  counts or brightness until the coloured shapes show.
- `LightEmission = 1` is additive (light), `0` is normal blending (matter).
  `Brightness` above 1 pushes a layer into bloom; check the place's
  `BloomEffect.Threshold`, a layer meant to glow has to clear it.
- `LightInfluence = 0` so an effect is not dimmed by a dark map.

## Particle properties worth knowing

- `Lifetime`, `Speed`, `SpreadAngle`, `Drag`, `Acceleration`: motion.
- `Size`, `Transparency`, `Color` sequences: shape over life.
- `Rotation`, `RotSpeed`: break up repetition; random start rotation.
- `Squash`: stretch a sprite into a streak without a new texture.
- `Orientation`: `FacingCamera` (default), `VelocityParallel` (streaks),
  `VelocityPerpendicular` (rings facing travel).
- `ZOffset`: push a layer in front of or behind others.
- `LockedToPart`: particles follow the emitter (auras, weapon glows).
- `TimeScale`: per-emitter speed; the Emit handle multiplies it for slow
  motion.
- `ShapeStyle`/`Shape`/`ShapeInOut`: emit from a volume, a surface, inward.
- Flipbooks: `FlipbookLayout` Grid2x2/4x4/8x8 on a 1024 sheet,
  `FlipbookMode` OneShot for bursts, Loop for fire.

## Textures

- White on transparent, so `Color` tints them; 256-512 px, flipbook sheets
  1024.
- Two families. **Drawn matter** (flames, slashes, puffs, shards) has a hard
  silhouette, detail from holes and notches, and is asymmetric. **Light**
  (flares, glows, soft rings) falls off softly and keeps its symmetry.
- Even spacing, mirror symmetry and uniform line width read as clip art.
  Warp coordinates and vary counts and widths.
- A soft grey smoke ball with no silhouette reads as a smudge at distance.
- Flipbook frames grow to a peak around a third of the way in, then break
  apart; leave a few pixels of gutter in each cell.
- Built-in textures (`rbxasset://textures/particles/...`) are fine stand-ins
  while timing a template.

## Beams, trails, meshes

- Beams and Trails need two attachments. Anything long and thin that joins
  two points is a Beam, not a particle.
- A Trail's `Lifetime` sets its length at a given speed; `WidthScale` and
  `Transparency` sequences taper it.
- Mesh effects (crescent, ring, dome, swirl) grow, spin and fade with a
  Start/End pair. They are open sheets: set `DoubleSided`. Neon material
  plus `Transparency` does most of the work.

## Playing it in a game

- Effects are presentation: play them on clients. The server fires an
  `UnreliableRemoteEvent` with the effect name and a CFrame; each client
  calls `Emit.play`. Never make damage or rewards wait on an effect.
- Tie the hit frame to the animation with a marker
  (`track:GetMarkerReachedSignal`).
- Move a projectile from the handle's `onStep`, so slow motion and captures
  see it where the effect says it is; start the impact with `handle:add`.

## Budgets

- Aim for a few hundred live particles for a big effect, tens for a hit.
- Each PointLight costs; one or two short ones per effect.
- Large half-transparent sprites that overlap fill the screen and cost
  fill-rate, especially on phones. Prefer fewer, shaped sprites.
- Pool effects that fire often instead of cloning every time.

## What to ask of each capture

- From the player's distance, does the payoff fill its share of the screen,
  or is it a speck?
- Is the flash one sharp shape, bigger than what follows, and gone by about
  0.15 s?
- At 0.3 s is a body still carrying the mass, or only sparks?
- Is there something dark so the bright parts read?
- Do drawn shapes have hard silhouettes, or are they smudges?
- Is the centre a white blob?
- Does anything show as a square or a box (a texture with no alpha, a whole
  flipbook sheet in one particle)?
- Does everything that moves carry its speed (streaks, a smear), or float?
- Past the end, is anything left behind?
