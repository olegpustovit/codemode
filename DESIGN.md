---
name: Codemode
description: A transparent, keyboard-driven control plane for connected tools and agent activity.
colors:
  ground: "#020309"
  surface: "transparent"
  text: "#F3F5FA"
  secondary-text: "rgba(243,245,250,.78)"
  quiet-text: "rgba(243,245,250,.58)"
  line: "rgba(200,215,255,.10)"
  strong-line: "rgba(200,215,255,.20)"
  accent-text: "color-mix(in srgb,rgb(var(--hue)) 40%,#fff)"
  accent-wash: "rgba(var(--hue),.16)"
  primary-fill: "rgba(var(--hue), .32)"
  control-hover: "rgba(255,255,255,.045)"
  error: "#FF8C98"
  warning: "#F3CE80"
typography:
  headline:
    fontFamily: "'M PLUS 1p', system-ui, sans-serif"
    fontSize: "28px"
    fontWeight: 300
    lineHeight: 1.15
    letterSpacing: "-.005em"
  title:
    fontFamily: "'M PLUS 1p', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 400
    lineHeight: 1.25
  body:
    fontFamily: "'M PLUS 1p', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.45
  label:
    fontFamily: "'M PLUS 1p', system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    letterSpacing: ".08em"
  code:
    fontFamily: "'JetBrains Mono', ui-monospace, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.6
rounded:
  square: "0px"
  keycap: "3px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "24px"
  "6": "32px"
  "7": "48px"
  "8": "72px"
components:
  button-primary:
    backgroundColor: "{colors.primary-fill}"
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    height: "30px"
    padding: "0 12px"
  button-ghost:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    height: "30px"
    padding: "0 12px"
  button-outline:
    backgroundColor: "{colors.ground}"
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    height: "30px"
    padding: "0 12px"
  button-danger:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.error}"
    rounded: "{rounded.square}"
    height: "30px"
    padding: "0 12px"
  search-field:
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    height: "32px"
    padding: "0 12px"
  section-navigation:
    textColor: "{colors.secondary-text}"
    rounded: "{rounded.square}"
    padding: "0 12px"
  policy-switch:
    rounded: "{rounded.square}"
  code-panel:
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
    padding: "8px 12px"
  work-pane:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.square}"
  keycap:
    textColor: "{colors.secondary-text}"
    rounded: "{rounded.keycap}"
    height: "18px"
    padding: "0 5px"
---

# Design System: Codemode

## Overview

**Creative North Star: "The approved transparent control plane"**

The owner-approved control-plane proposal is the visual authority: [proposal](https://artifacts.microapps.space/a/hkwwwp90n0dt), preserved in `docs/proposal/control-plane.html`. Its final v5 overrides supersede the earlier daytime design. This document records the implemented replacement; release qualification is recorded separately in `docs/proposal/implementation.md`.

The interface is an Operate surface. Compact lists, readable code, explicit connection states and a visible path through stacked panes carry the work. A continuous ribbon shader sits behind transparent work regions, with cobalt, violet, teal, bronze and crimson section hues. Raster decoration is not used.

**Key Characteristics:**

- Transparent square work surfaces
- One moving list cursor and tab underline
- Self-hosted M PLUS 1p and JetBrains Mono
- Visible state and precise recovery

## Colors

### Primary

Section-tinted light colors focus, selection and active navigation. The hue follows Sessions (cobalt), Integrations (violet), Workspaces (teal), Scripts (bronze) and Tokens (crimson), as defined by `Backdrop`. The primary button uses its stronger fill; the accent wash remains a separate selection token.

### Neutral

Night ground, transparent work surfaces, light text and fine rules use the normative frontmatter values and `ui/src/index.css`. Text has three levels: primary identity, secondary working content and quieter metadata. Fine rules separate regions without darkening their fills. Accent text, cursor wash, focus and tab underline derive from the current section's `--hue`. Rose indicates failures and blocks; amber marks attention. The unused success primitive remains in the stylesheet and is not a current state treatment.

**The Final Override Rule.** The final ground, transparent surfaces, line opacities and accent mix override the earlier base declarations in the stylesheet. Do not copy the earlier base declarations in isolation.

## Typography

M PLUS 1p is self-hosted at 300, 400 and 500 for interface text. JetBrains Mono is self-hosted at 400 and 500 for Python, identifiers, timestamps, measurements and keyboard hints. Latin and Latin Extended M PLUS subsets are bundled; other scripts fall back to system fonts. The proposal’s document-only display samples are not a shipped application type role.

The stylesheet defines an 11, 12, 13, 14, 16, 20, 28, 40 and 64px scale; its two largest sizes are not normal application headings. Page headings are 28px/300. Pane titles are 20px/400; tool and script titles use 17px/400 mono at 1.4 line height. List rows use 14px; controls and metadata use 11–13px. Functional column, group and code labels use 11px/500 with .08em tracking; code panels use 12px mono at 1.6 line height. Keep long identifiers wrapped in details and ellipsized in compact rows.

## Layout

The desktop shell has a 48px top bar, 228px rail and 30px status bar. Lists begin at full width. Two-pane screens use 46/54 proportions. Sessions use 36/64, then 20/32/48 when an execution opens. Detail headings stay above their scrolling bodies; list column headings stay above independently scrolling rows. Standard rows are 40px; timeline lanes are 44px.

The 4/8/12/16/24/32/48/72px spacing scale governs the composition. At 1000px and below, the top pane occupies the available width. At 767px and below, search wraps into the top bar's second row, navigation remains visible as a horizontal section strip, and less important columns disappear. Source configuration, OAuth consent, authentication and script editing retain the same typography, square controls and ruled sections.

## Elevation & Depth

Depth comes from pane widths, fine boundary rules and the shared shader. Keep work regions transparent. **The Transparent Work Region Rule.** Work panes stay transparent; small control and code backings retain their observed washes. The selected list cursor and current section have a restrained hue wash; key focus and selected boundaries use small accent glows. Text uses a slight offset shadow for legibility over the wave.

The shader pauses in hidden tabs and renders one static frame for reduced motion, with a gradient fallback if WebGL cannot initialize. It caps device-pixel ratio at 2 and releases its resources on unmount. Hardware frame-rate and idle-CPU acceptance remain external checks, not design-document claims.

## Shapes

Work surfaces, buttons, inputs and code panels have square corners. Keycaps alone use 3px corners and a stronger lower rule. States are plain words rather than filled pills. The shared grid has one gliding cursor; tabs share one moving underline. Vendor illustrations use the supplied ring and tick geometry and Simple Icons paths. Custom sources use an SVG plug. The prototype's initial-letter fallback remains in the implementation for vendors without paths; it is not a reusable icon rule.

## Components

### Controls

Buttons are square, 30px high with 12px horizontal padding and 13px medium UI text. Primary actions have a section-tinted border and primary fill; hover increases that fill and adds the larger glow. Ghost actions stay transparent at rest. Outline actions use the ground fill with a rule; destructive text actions use a rose border and transparent fill. The shared keyboard focus treatment is a two-pixel accent outline offset by two pixels. Disabled controls reduce opacity.

Search is a 32px square field with a faint white wash, a fine rule, a 14px SVG and 13px UI input. Focus strengthens the rule, wash and small glow. Policy switches use a square 26×14px track and 9×8px slider; blocked state moves the slider left and colors it rose. Code panels use a dark translucent backing and a two-pixel left rule rather than a containing card. There are no reusable filled chips or cards.

### Navigation and panes

**The One Cursor Rule.** Each data grid shares one moving selection cursor; each tab bar shares one underline.

- **AppShell:** five sections in order: Sessions, Integrations, Workspaces, Scripts, Tokens. The rail shows workspace selection and counts; the responsive top bar retains the workspace chooser. A paginated session count uses `50+` when more sessions exist. Number keys 1–5 switch sections outside editable controls. Workspace selection carries between Workspaces and Scripts.
- **Breadcrumbs:** begin with the selected workspace and retain section, item and execution context; ancestors are navigation targets.
- **DataList:** grouped compact rows, one grid focus and active descendant, fixed column headings, Arrow/Home/End navigation, Enter/right to open and left/Escape to return. Keep the cursor inside the scrolling body.
- **TabBar:** one selected tab and underline; arrow/Home/End keys move tab focus. Brackets switch multi-tab screens.
- **PaneStack:** animate width and opacity. Mobile shows the deepest pane and restores focus on return. Closed panes cannot take focus. URL state retains selection and depth through refresh and browser history.
- **Sessions:** list or scaled rolling 96-hour timeline; V switches layouts. Real execution ticks are loaded as paginated metadata. Compact metadata shows duration and time until expiry, with four summary values and an explicit rename action; no retention progress bar. The execution pane exposes code, result, stdout and redacted call previews, with copy and save-as-script actions after the execution evidence.
- **Workspaces:** tabs select workspaces; grouped tool rows expose allowed/blocked switches. Details show the stored Python signature and source-wide policy actions. Tool/source scopes remain distinct from policy overrides.
- **Scripts:** list and preview pane lead to the existing revision-protected editor. Deletion requires confirmation and preserves execution history.
- **Tokens:** agent list, details and personal-token creation pane. New secrets are shown once and disappear when the pane closes. Revocation uses the shared confirmation control.
- **StatusBar / ConfirmAction:** errors and results appear in the bottom bar. Destructive actions require a second press within three seconds; another status action invalidates the pending confirmation. No modal or toast layer is used.

## Do's and Don'ts

### Do:

- Do follow the exact proposal and final v5 overrides when extending the system.
- Do preserve keyboard focus, reduced motion, tenant boundaries, write-only secrets and precise errors.
- Do show real connection and execution state; use explicit loading, empty and failure treatments.

### Don't:

- Don't reintroduce opaque cards, rounded work surfaces, pills, sidebar sheets, toasts or approval dialogs.
- Don't replace the proposal shader, fonts, glyph geometry or section hues with a new direction.
- Don't claim provider consent, external-client acceptance or M-series performance from local fixtures.
