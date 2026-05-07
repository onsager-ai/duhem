# Duhem — Brand Marks

> **Status**: v0.1
> **Last updated**: 2026-05-07

This document specifies the Duhem brand mark, its design rationale, its relationship to its sister product Onsager, and its usage guidelines.

-----

## 1. The mark

The Duhem mark is a continuous square frame surrounding a centered solid square, on a 32×32 unit grid.

<p align="center">
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="128" height="128" role="img" aria-label="Duhem">
    <title>Duhem</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
</p>

### Geometric specification

|Element      |Position|Size   |
|-------------|--------|-------|
|Top bar      |(2, 2)  |28 × 4 |
|Bottom bar   |(2, 26) |28 × 4 |
|Left bar     |(2, 6)  |4 × 20 |
|Right bar    |(26, 6) |4 × 20 |
|Center square|(11, 11)|10 × 10|

The frame thickness is 4 units; the gap between frame and center is 5 units; the center square is 10 units.

## 2. Design rationale

The mark visualizes the Duhem-Quine thesis after which the platform is named.

The **center square** is the hypothesis under test — the AI-generated artifact, the code, the unit a naive verification approach would test in isolation.

The **frame** is the auxiliary web — prompts, configurations, runtime context, tool wiring, data state, upstream service contracts. Everything that surrounds and supports the hypothesis. Without the frame, the hypothesis cannot be evaluated. With the frame present but unstated, evaluation becomes circular.

The **gap** between frame and center is the conceptual space where component testing tries (and fails) to operate — pretending the center is decomposable from the frame.

The **whole figure** is what Duhem actually verifies: hypothesis and web together, as one system.

The mark is solid, geometric, mathematical. It carries the gravity of a scientist’s thesis without illustrating it literally.

## 3. Relationship to Onsager

Duhem is the sister product to Onsager. The marks share visual DNA, signaling the family relationship.

<p align="center">
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="96" height="96" role="img" aria-label="Onsager">
    <title>Onsager</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="7" height="7"/>
      <rect x="23" y="2" width="7" height="7"/>
      <rect x="2" y="23" width="7" height="7"/>
      <rect x="23" y="23" width="7" height="7"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
  &nbsp;&nbsp;&nbsp;&nbsp;
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="96" height="96" role="img" aria-label="Duhem">
    <title>Duhem</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
</p>

### Shared DNA

- Both marks live on a 32×32 unit grid.
- Both are monochrome solid forms — no gradients, no decoration.
- **Both share the identical center square**: 10×10 at position (11, 11).
- Both carry a scientist’s surname and a thesis behind it.

### Distinguishing form

- **Onsager** has four discrete corner blocks (7×7 each, at the corners). Read as: lattice points, discrete interactions.
- **Duhem** has a continuous unbroken frame. Read as: a connected web, holistic boundary.

### Why this relationship matters

The form difference reflects the thesis difference:

- Onsager solved the **Ising model** — a discrete lattice where each site interacts with its neighbors. Corner blocks visualize the discrete-point character of that physics.
- The **Duhem-Quine thesis** is about confirmation holism — that hypotheses are tested as parts of a continuous web of theoretical and auxiliary commitments. The continuous frame visualizes the inseparable web.

The marks are not arbitrary geometry. They are minimal geometric instances of what each scientist taught us.

This is a design discipline we hold across the family: **form follows thesis**. Future products in this family should follow the same rule — pick a thesis, find its minimal geometric instance.

## 4. Sizing

The mark is designed at 32×32 base resolution and scales cleanly to common UI sizes.

<p align="center">
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="16" height="16" role="img" aria-label="Duhem at 16px">
    <title>Duhem 16px</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
  &nbsp;&nbsp;
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="24" height="24" role="img" aria-label="Duhem at 24px">
    <title>Duhem 24px</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
  &nbsp;&nbsp;
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32" role="img" aria-label="Duhem at 32px">
    <title>Duhem 32px</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
  &nbsp;&nbsp;
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48" role="img" aria-label="Duhem at 48px">
    <title>Duhem 48px</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
  &nbsp;&nbsp;
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="64" height="64" role="img" aria-label="Duhem at 64px">
    <title>Duhem 64px</title>
    <g fill="currentColor">
      <rect x="2" y="2" width="28" height="4"/>
      <rect x="2" y="26" width="28" height="4"/>
      <rect x="2" y="6" width="4" height="20"/>
      <rect x="26" y="6" width="4" height="20"/>
      <rect x="11" y="11" width="10" height="10"/>
    </g>
  </svg>
</p>

|Size |Use case                                   |
|-----|-------------------------------------------|
|16px |Browser favicon, tiny inline icons         |
|24px |Standard UI icon (toolbar, list items)     |
|32px |Master canvas; app icon at standard density|
|48px |App icon at 1.5× density; navigation marks |
|64px+|Display use; hero mark; print              |

### Minimum legible size

The mark holds form down to 16px. Below 16px, the 4-unit frame thickness and 5-unit gap collapse to single pixels, losing the frame-vs-center distinction.

For sub-16px contexts, use a simplified favicon (frame only, no center) or skip the mark.

## 5. Color

The default mark is monochrome: solid black on white in light contexts; solid white on black in dark contexts.

<p align="center">
  <span style="display: inline-block; padding: 16px; background: #ffffff;">
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="64" height="64" style="color: #000000" role="img" aria-label="Duhem on light">
      <title>Duhem on light</title>
      <g fill="currentColor">
        <rect x="2" y="2" width="28" height="4"/>
        <rect x="2" y="26" width="28" height="4"/>
        <rect x="2" y="6" width="4" height="20"/>
        <rect x="26" y="6" width="4" height="20"/>
        <rect x="11" y="11" width="10" height="10"/>
      </g>
    </svg>
  </span>
  &nbsp;
  <span style="display: inline-block; padding: 16px; background: #000000;">
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="64" height="64" style="color: #ffffff" role="img" aria-label="Duhem on dark">
      <title>Duhem on dark</title>
      <g fill="currentColor">
        <rect x="2" y="2" width="28" height="4"/>
        <rect x="2" y="26" width="28" height="4"/>
        <rect x="2" y="6" width="4" height="20"/>
        <rect x="26" y="6" width="4" height="20"/>
        <rect x="11" y="11" width="10" height="10"/>
      </g>
    </svg>
  </span>
</p>

The mark uses `currentColor` for fill, inheriting from CSS context. This means:

- In any container with `color: #000`, mark renders black.
- In dark mode (`color: #fff`), mark renders white.
- No SVG re-export needed for color variation.

### Tinted contexts

For UI surfaces that wrap the mark in a tinted background (e.g., navigation cards, app shelves), the mark should sit on a `bg-primary/10` (10% opacity primary) container at the appropriate size, mirroring how Onsager appears in similar contexts:

<p align="center">
  <span style="display: inline-flex; align-items: center; justify-content: center; width: 48px; height: 48px; background: rgba(0,0,0,0.10); border-radius: 8px;">
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="24" height="24" style="color: #000000" role="img" aria-label="Duhem in tinted card">
      <title>Duhem in tinted card</title>
      <g fill="currentColor">
        <rect x="2" y="2" width="28" height="4"/>
        <rect x="2" y="26" width="28" height="4"/>
        <rect x="2" y="6" width="4" height="20"/>
        <rect x="26" y="6" width="4" height="20"/>
        <rect x="11" y="11" width="10" height="10"/>
      </g>
    </svg>
  </span>
</p>

### Accent palette (for marketing surfaces)

Where a single accent color is desirable (web hero, marketing collateral), use a deep, neutral hue with engineering character:

|Use              |Hex    |Note                                             |
|-----------------|-------|-------------------------------------------------|
|Primary mark     |#000000|Default monochrome                               |
|Inverse mark     |#FFFFFF|Dark contexts                                    |
|Accent (optional)|#1E3A5F|Deep navy — gravity, scientific instrument flavor|
|Accent alt       |#0F4C3A|Forest green — alternative for visual variety    |

Accent colors are optional. The default and dominant treatment is monochrome.

## 6. Wordmark

The wordmark spelling is **Duhem** — title case, no all-caps in standard usage, no italics, no embellishment.

Recommended typeface: a geometric sans with mathematical character. Suitable choices:

- **Inter** (400 weight) — clean, neutral, modern
- **Geist Sans** (400 weight) — Vercel’s typeface; geometric with subtle warmth
- **JetBrains Mono** (when monospaced flavor desired) — engineering-coded contexts
- **IBM Plex Sans** (400 weight) — institutional gravity

Weight: 400 (regular) for body and lockup; never bolder than 500.
Tracking: default to slightly tight (-0.5%) at display sizes; default at body sizes.
Case: title case in lockup (“Duhem”), all lowercase in code or path contexts (“duhem”).

### Pronunciation

“DOO-em” (English approximation), or “duˈɛm” (closer to French original). Two syllables, primary stress on first.

## 7. Lockup

The standard lockup places the mark to the left of the wordmark, vertically center-aligned, with the gap equal to the mark’s frame thickness scaled to the wordmark cap height.

<p align="center">
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 240 64" width="240" height="64" role="img" aria-label="Duhem lockup">
    <title>Duhem lockup</title>
    <g fill="currentColor">
      <rect x="8" y="16" width="28" height="4"/>
      <rect x="8" y="40" width="28" height="4"/>
      <rect x="8" y="20" width="4" height="20"/>
      <rect x="32" y="20" width="4" height="20"/>
      <rect x="17" y="25" width="10" height="10"/>
    </g>
    <text x="56" y="42" font-family="Inter, system-ui, sans-serif" font-size="28" font-weight="400" fill="currentColor" letter-spacing="-0.4">Duhem</text>
  </svg>
</p>

### Spacing rules

- Mark height = wordmark cap height (visual alignment).
- Gap between mark and wordmark = mark frame thickness × (wordmark cap height / mark canvas size).
- Vertical alignment: optical center (typically wordmark x-height aligned to mark vertical center).

### Lockup variants

- **Horizontal lockup** (above): default for headers, navigation, business cards.
- **Vertical lockup** (mark on top, wordmark below): for square contexts (app launchers, OG images).
- **Mark-only**: for favicons, tight UI contexts, repeated brand presence.
- **Wordmark-only**: for body-text references, footer credits, contexts where the mark would be redundant.

## 8. Usage guidelines

### Do

- Use `currentColor` so the mark inherits theme color.
- Maintain proportions — never stretch or skew.
- Preserve clear space — at minimum, half the mark’s width on all sides.
- Use the mark at sizes ≥ 16px.
- Pair the mark with the wordmark in first-impression contexts.

### Don’t

- Don’t change the geometry — frame thickness, center size, and proportions are load-bearing.
- Don’t add gradients, shadows, glows, or 3D effects.
- Don’t rotate the mark.
- Don’t enclose the mark in a circle or other container shape (the frame is already a container; nesting confuses the meaning).
- Don’t use the mark on busy or photographic backgrounds without sufficient contrast.
- Don’t recolor the center square independently of the frame.

## 9. Implementation

### Inline SVG (recommended)

```html
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="32" height="32" role="img" aria-label="Duhem">
  <title>Duhem</title>
  <g fill="currentColor">
    <rect x="2" y="2" width="28" height="4"/>
    <rect x="2" y="26" width="28" height="4"/>
    <rect x="2" y="6" width="4" height="20"/>
    <rect x="26" y="6" width="4" height="20"/>
    <rect x="11" y="11" width="10" height="10"/>
  </g>
</svg>
```

### Favicon (32×32 PNG)

Export the SVG to PNG at 32×32 with transparent background. Provide additional sizes (16, 48, 64, 192, 512) for various platforms.

### App icon

Use the mark on a solid-color rounded-square plate following the platform’s icon conventions (iOS: 60×60 → 1024×1024; Android: 192×192 → 512×512). Mark occupies ~60% of plate width, centered.

## 10. Family roadmap

The shared 10×10 center square at (11, 11) is the family signature. Future products in the family should preserve this signature while finding their own thesis-derived outer geometry.

Hypothetical examples (illustrative, not committed):

- A product about graph topology might use **vertices and edges** as the outer geometry.
- A product about flow systems might use **directed arrows** as the outer geometry.
- A product about partitions might use **bisecting lines** as the outer geometry.

The family rule: **outer form follows thesis; center remains fixed**.

-----

## Appendix A — File assets

The brand mark is provided in the following formats. (Exports are produced from a single SVG source of truth.)

|File                       |Use                                |
|---------------------------|-----------------------------------|
|`duhem.svg`                |Master vector, scalable to any size|
|`duhem-32.png`             |Standard UI icon                   |
|`duhem-favicon.ico`        |Browser favicon                    |
|`duhem-app-icon.png`       |App icon (1024×1024 base)          |
|`duhem-lockup.svg`         |Horizontal lockup with wordmark    |
|`duhem-lockup-vertical.svg`|Vertical lockup                    |
|`duhem-mark-only.svg`      |Mark without wordmark              |
|`duhem-wordmark.svg`       |Wordmark without mark              |

## Appendix B — Reserved variants

The following variants are reserved for specific contexts and should not be used outside them.

- **Loading state animation**: the center square pulses (0.8 → 1.0 opacity) at 1.5s cycle. Used in CLI and dashboard while a verification run is in progress.
- **Failure state mark**: center square inverted to `currentColor: var(--color-danger)` while frame remains primary. Used only in verdict-display contexts, never as standalone brand presence.
- **Success state mark**: center square set to `currentColor: var(--color-success)` while frame remains primary. Same usage rule.

These are display states, not brand variants. They communicate run state, not identity.
