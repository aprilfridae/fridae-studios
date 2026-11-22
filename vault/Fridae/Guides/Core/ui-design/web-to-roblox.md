# Using the web design guides in Roblox

rbxkit bundles seven general design guides (from the open-design project).
They are written for websites and CSS, but the judgement in them carries
over to Roblox UI and effects. Read them for *what good looks like*; use this
page to turn their CSS into Roblox.

## Which guide for what

| Need | Guide (skill name / rbx_guide topic) |
|---|---|
| Polish; deciding whether and how something should animate; reviewing motion | `emil-design-eng` |
| Trailers, cutscenes, effect timing, beat maps | `motion-design` |
| Auditing and upgrading UI that already exists | `redesign-skill` |
| Matching a reference image or "make it feel like this" | `reference-design-contract` |
| A soft, premium, glassy look | `soft-skill` |
| A clean, quiet, minimal look | `minimalist-skill` |
| A raw, industrial, terminal look | `brutalist-skill` |

Pick one style guide per game and stay in it. A supplied reference or the
game's existing UI still wins over any of them.

## CSS to Roblox

| Web | Roblox |
|---|---|
| `transform: scale()` | A `UIScale` child; tween `UIScale.Scale`. Never tween `Size` for press or pop feedback |
| `transform: translate()` | Tween `Position`, or an offset on a wrapper Frame |
| `opacity` | `GroupTransparency` on a `CanvasGroup`, or each object's own `BackgroundTransparency` / `TextTransparency` / `ImageTransparency` |
| `transition: 200ms ease-out` | `TweenInfo.new(0.2, Enum.EasingStyle.Quad, Enum.EasingDirection.Out)` |
| `cubic-bezier(0.32, 0.72, 0, 1)` (fast start, long settle) | `Enum.EasingStyle.Quint` or `Exponential`, `Out` |
| ease-in-out | `Sine` or `Quad`, `InOut`. Keep for things that move across the screen, not for entrances |
| Spring | `Enum.EasingStyle.Back` `Out` for a small overshoot, or a spring module if the game has one |
| `border-radius` | `UICorner` (inner radius = outer radius minus the padding, for nested shapes) |
| `border` / hairline | `UIStroke`, `ApplyStrokeMode = Border`, with `Transparency` for a soft line |
| `box-shadow` | No native shadow: a blurred 9-slice shadow image behind the element, tinted to the background hue |
| `linear-gradient` | `UIGradient` (`Color`, `Transparency`, `Rotation`) |
| `backdrop-filter: blur` | Not available per element. A `BlurEffect` in Lighting blurs the whole world behind a full-screen menu; otherwise use a semi-opaque fill |
| `letter-spacing` | Not available. Get the same weight contrast from font weight and size |
| `font-variant-numeric: tabular-nums` | Use a monospace `FontFace` for counters that tick |
| `max-width` | `UISizeConstraint.MaxSize` |
| `aspect-ratio` | `UIAspectRatioConstraint` |
| flexbox / grid | `UIListLayout` (with `UIFlexItem`), `UIGridLayout`, `UIPadding` |
| `:hover` | `MouseEnter` / `MouseLeave`; skip it on touch (`UserInputService.TouchEnabled`) |
| `:active` | `InputBegan` / `InputEnded`, or tween on `Activated` |
| `:focus-visible` | `GuiService.SelectedObject` and `SelectionImageObject` for gamepad |
| `prefers-reduced-motion` | `GuiService.ReducedMotionEnabled` |
| `z-index` | `ZIndex` with `ZIndexBehavior.Sibling`, `DisplayOrder` per ScreenGui |
| `overflow: hidden` | `ClipsDescendants`; rounded clipping needs a `CanvasGroup` |
| `@media` breakpoints | Read `workspace.CurrentCamera.ViewportSize`, or one `UIScale` per window driven by it |
| noise / grain overlay | A tiled noise image at high transparency, `ScaleType.Tile` |

## Rules that change in a game

- **Readability beats elegance.** UI sits over a moving 3D world. Text needs
  a stroke or a solid plate; hairline type that works on a web page vanishes.
- **Touch first.** Many players are on phones: 44 px targets, no hover-only
  information, keep the thumbstick and jump areas clear.
- **Web advice to ignore:** semantic HTML, meta tags, cookie banners, 404
  pages, font loading, scroll-driven effects, `100dvh`.
- **Fonts:** only what Roblox ships (`Font.fromName`, `Enum.Font`) or the
  game's uploaded fonts. "Not Inter" still applies in spirit: don't leave the
  default font.
- **Motion budget:** a tween per frame per element is fine; hundreds of
  simultaneous tweens or per-frame layout changes are not.
- **Durations still hold:** 100-160 ms for press feedback, 150-250 ms for
  small UI, 300-500 ms for windows, and exits faster than entrances.
- **Never pop from scale 0.** Start at about 0.9-0.95 with transparency.
