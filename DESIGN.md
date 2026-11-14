---
name: Codemode
description: A software distribution index for connected tools and endpoint scope.
colors:
  background: "oklch(1 0 0)"
  foreground: "oklch(0.145 0 0)"
  card: "oklch(1 0 0)"
  card-foreground: "oklch(0.145 0 0)"
  popover: "oklch(1 0 0)"
  popover-foreground: "oklch(0.145 0 0)"
  primary: "oklch(0.205 0 0)"
  primary-foreground: "oklch(0.985 0 0)"
  muted: "oklch(0.97 0 0)"
  muted-foreground: "oklch(0.47 0.012 255)"
  destructive: "oklch(0.48 0.19 27.325)"
  border: "oklch(0.922 0 0)"
  input: "oklch(0.922 0 0)"
  ring: "oklch(0.708 0 0)"
  sidebar: "oklch(0.25 0.035 255)"
  sidebar-foreground: "oklch(0.96 0.006 255)"
  sidebar-accent: "oklch(0.34 0.045 255)"
  sidebar-accent-foreground: "oklch(1 0 0)"
  sidebar-border: "oklch(0.39 0.035 255)"
  sidebar-ring: "oklch(0.708 0 0)"
  selection: "oklch(.86 .045 255)"
  selection-foreground: "oklch(.2 .03 255)"
typography:
  headline:
    fontFamily: "'Geist Variable', sans-serif"
    fontSize: "1.75rem"
    fontWeight: 650
    lineHeight: 1.2
    letterSpacing: "-0.025em"
  title:
    fontFamily: "'Geist Variable', sans-serif"
    fontSize: "1.125rem"
    fontWeight: 600
    lineHeight: 1.5
  body:
    fontFamily: "'Geist Variable', sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.6
  label:
    fontFamily: "'Geist Variable', sans-serif"
    fontSize: "0.875rem"
    fontWeight: 500
    lineHeight: "1.25rem"
  metadata:
    fontFamily: "'Geist Variable', sans-serif"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: "1rem"
  code:
    fontFamily: "ui-monospace, SFMono-Regular, monospace"
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: 1.65
rounded:
  md: "0.5rem"
  lg: "0.625rem"
  xl: "0.875rem"
  4xl: "1.625rem"
spacing:
  "1": "0.25rem"
  "1.5": "0.375rem"
  "2": "0.5rem"
  "2.5": "0.625rem"
  "3": "0.75rem"
  "4": "1rem"
  "5": "1.25rem"
  "6": "1.5rem"
  "7": "1.75rem"
  "9": "2.25rem"
  "10": "2.5rem"
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.primary-foreground}"
    typography: "{typography.label}"
    rounded: "{rounded.lg}"
    height: "2rem"
    padding: "0 0.625rem"
  button-outline:
    backgroundColor: "{colors.background}"
    textColor: "{colors.foreground}"
    typography: "{typography.label}"
    rounded: "{rounded.lg}"
    height: "2rem"
    padding: "0 0.625rem"
  button-outline-hover:
    backgroundColor: "{colors.muted}"
    textColor: "{colors.foreground}"
  button-ghost:
    backgroundColor: "transparent"
    typography: "{typography.label}"
    rounded: "{rounded.lg}"
    height: "2rem"
    padding: "0 0.625rem"
  button-destructive:
    backgroundColor: "color-mix(in oklab, var(--destructive) 10%, transparent)"
    textColor: "{colors.destructive}"
    typography: "{typography.label}"
    rounded: "{rounded.lg}"
    height: "2rem"
    padding: "0 0.625rem"
  button-destructive-hover:
    backgroundColor: "color-mix(in oklab, var(--destructive) 20%, transparent)"
  input:
    backgroundColor: "transparent"
    textColor: "{colors.foreground}"
    rounded: "{rounded.lg}"
    height: "2rem"
    padding: "0.25rem 0.625rem"
  navigation:
    backgroundColor: "{colors.sidebar}"
    textColor: "{colors.sidebar-foreground}"
    typography: "{typography.label}"
    width: "16rem"
  state-badge:
    textColor: "{colors.foreground}"
    rounded: "{rounded.4xl}"
    height: "1.25rem"
    padding: "0.125rem 0.5rem"
  state-badge-destructive:
    backgroundColor: "color-mix(in oklab, var(--destructive) 10%, transparent)"
    textColor: "{colors.destructive}"
    rounded: "{rounded.4xl}"
    height: "1.25rem"
    padding: "0.125rem 0.5rem"
  code-panel:
    backgroundColor: "{colors.muted}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.lg}"
    padding: "1rem"
  scope-summary:
    backgroundColor: "{colors.muted}"
    textColor: "{colors.foreground}"
    rounded: "{rounded.lg}"
    padding: "1rem"
---

# Design System: Codemode

## Overview

**Creative North Star: "Software distribution index"**

The control plane pairs a slate-blue navigation rail with white work surfaces, compact controls and visible record state. Geist supplies the interface voice; borders and spacing organize tables, forms and execution evidence.

This records the implemented daytime interface. The direction is an implementation assumption documented in the surface brief, without recorded user aesthetic approval. Generated dark-mode variables exist in the stylesheet, but the application exposes no theme switch; they are outside this recorded active palette.

**Key Characteristics:**

- Slate-blue navigation and white work surfaces.
- Compact controls with explicit text labels and status.
- Tables, ruled sections and responsive detail columns.
- Endpoint membership shown as a live scope summary.

## Colors

Cool slate frames a neutral working palette. Frontmatter values preserve the stylesheet's normative OKLCH notation.

### Primary

- **Charcoal action** (`primary`, `primary-foreground`): filled save, add, sign-in and authorize controls.
- **Slate navigation** (`sidebar`, `sidebar-foreground`): persistent navigation; the lighter `sidebar-accent` and its foreground identify active and hovered items. `sidebar-border` and `sidebar-ring` support its boundaries and focus.
- **Destructive red** (`destructive`): removal actions, failed states, validation and error notices. Destructive buttons and badges use translucent red fills; they retain red text.

### Neutral

- **White work surface** (`background`, `card`, `popover` and their foregrounds): page canvas, alerts and overlay content.
- **Quiet gray** (`muted`, `muted-foreground`): code and scope panels, hover fills, descriptions and metadata.
- **Pale boundary** (`border`, `input`): table rules, section rules and control outlines. `ring` marks keyboard focus.
- **Blue selection** (`selection`, `selection-foreground`): browser text selection. Carets use `primary`; scrollbars use `border` over transparency.

**The State in Words Rule.** Status badges always name the state; red accompanies error states rather than replacing their label.

## Typography

**Heading and Body Font:** self-hosted Geist Variable, with sans-serif fallback. The existing rendered-font capture confirms the custom Geist face rendered.

**Code Font:** platform monospace for Python, JSON, namespaces, endpoint addresses and execution identifiers.

### Hierarchy

- **Headline:** page titles use the frontmatter headline role with balanced wrapping.
- **Title:** section headings use the title role; the brand uses the same size and weight.
- **Body:** descriptive paragraphs use the body role. Page-heading descriptions stop at the observed text container width (42rem).
- **Label:** controls, navigation, table content and field labels use the compact label size. Table content and helper copy use regular weight; field labels use medium weight and snug leading.
- **Metadata:** secondary namespace lines and compact record details use the metadata role. State badges use its size with medium weight.
- **Code:** preformatted evidence uses the code role and wraps long content. Numeric tool counts and durations use tabular numerals.

**The Data Type Rule.** Monospace identifies code and machine identifiers; ordinary interface copy uses Geist.

## Layout

The desktop shell has a left navigation rail (16rem) and a ruled top bar (3.5rem high). The rail defaults open and can move off canvas. Work content fills the remaining width, centers within its maximum width (80rem), and uses mobile padding (1.25rem), increasing at the medium breakpoint to desktop padding (2.25rem). The main vertical gap is 1.75rem.

At widths below 48rem, navigation opens in a left sheet (18rem wide); choosing a route closes it. Heading and action groups wrap. Tables retain their columns inside a horizontal scrolling container. The viewport minimum is 20rem, and long identifiers can break within the content area.

Detail pages stack until the large breakpoint (64rem), then use two columns with a 2:1 ratio, a minimum aside width (15rem) and a gap (2.5rem). Forms stop at 42rem and separate field groups by 1.25rem. The sign-in panel stops at 28rem, with padding and gaps of 1.75rem.

Spacing follows the inherited quarter-rem rhythm, including half steps for compact controls. Table headings are 2.5rem high; table cells use 0.5rem padding. Heading descriptions begin 0.5rem below their title.

## Elevation & Depth

Working records are flat, divided by fine rules and muted tonal panels. The sign-in panel uses the small ambient shadow. Sheets use the larger overlay shadow; confirmation dialogs use a faint foreground outline (1px). These are soft, offset shadows; focus rings signal keyboard state separately. Exact shadow values and transition timings live in the sidecar.

The mobile sheet uses a restrained dark backdrop with a small supported backdrop blur. Its transition lasts 200ms; sheet-backdrop opacity and ordinary control transitions last 150ms. Confirmation dialogs use a 100ms fade and subtle zoom. Reduced-motion preference shortens animation and transition durations to 0.01ms. No page-entry sequence is implemented.

**The Flat Records Rule.** Record lists and detail sections use borders and spacing for separation; elevation belongs to the sign-in panel and overlays.

## Shapes

Controls and code panels use gently rounded corners from the large radius. Navigation items use the medium radius; the sign-in panel uses the extra-large radius. State badges use the largest recorded radius on a short capsule. Fine single-pixel borders define inputs, tables, panels and overlays.

Icons are Lucide SVGs: ordinary control icons are 1rem; the brand mark is 1.5rem. Compact controls keep icon and label alignment consistent.

## Components

### Buttons

Compact Base Nova controls. Primary buttons use charcoal with a light foreground; outline controls use a white fill and pale border. Ghost controls inherit their context's text color. Destructive controls use red text over translucent red. Default sizing, padding and corners are in frontmatter; icon buttons are square (2rem).

Primary hover reduces its fill to 80% opacity; outline and ghost hover use the muted fill. Destructive hover doubles its red fill opacity. Keyboard focus changes the border to `ring` and adds a translucent ring (3px). Pressed buttons move down 1px unless they expose a popup. Disabled controls stop pointer interaction and use half opacity.

### Chips

State badges are compact text capsules. Normal states use the outline treatment; `error`, `failed`, `timed_out` and `blocked` use the destructive treatment. Underscores become spaces in the visible state name. They are informational labels in the shipped record views.

### Cards / Containers

Code and scope panels use a muted fill, fine border, large radius and 1rem padding. Code panels wrap and scroll overflowing evidence. The sign-in panel is a centered white bordered surface with an extra-large radius and small shadow. Ordinary tables and detail sections sit directly on the canvas.

### Inputs / Fields

Inputs have transparent fills and a pale outline. Input text is 1rem on mobile, changing to 0.875rem at the medium breakpoint. Placeholders and descriptions use `muted-foreground`; visible labels sit above fields. Focus uses the same 3px ring as buttons. Invalid controls use a destructive border and translucent red ring. Disabled controls combine reduced opacity, an input-colored fill and disabled interaction.

Error notices pair a problem title with a message inside a bordered white alert. Field errors use destructive text. Empty states provide a title, explanation and an action where applicable; loading views use labeled skeletons.

### Navigation

The rail combines brand, hostname, six labeled destinations and a footer sign-out action. Active routes use the lighter slate fill and medium weight. Hover shares this fill; focus uses the sidebar ring (2px). Compact icons accompany the labels. The top-bar trigger toggles the desktop rail or mobile sheet. A keyboard-visible skip link reaches the main content.

### Current Scope Summary

Endpoint editing includes a muted bordered summary beneath source selection. It names selected source slugs and totals their available tools; an empty selection explicitly states that agents will see no tools. The section uses a polite live region so the current scope updates accessibly as selection changes.

### Tool Scopes and GraphQL

Tool policies include an all/selected scope mode, per-tool inclusion controls and a textual outside-scope state. Desktop uses a ruled table; mobile stacks each record with its name, inclusion, effective policy and override. GraphQL reuses the source connection selector and HTTP credential form. The visual tokens and six navigation destinations are unchanged.

## Do's and Don'ts

### Do:

- **Do** reuse the shared controls, page headings, status badges and ruled detail sections.
- **Do** pair record state with readable text and keep the corrective action near its context.
- **Do** preserve visible keyboard focus, wrapped identifiers and the mobile navigation sheet.
- **Do** show endpoint membership and tool count together in the current scope summary.

### Don't:

- **Don't** replace status words with color alone.
- **Don't** use monospace for ordinary headings, navigation or descriptive copy.
- **Don't** wrap each record section in an elevated card; retain the flat record structure.

## Account registration extension

Registration and sign-in reuse the centered auth panel and existing Field, Input, Button and ErrorNotice components. Email is shown in the sidebar footer for account identification. Desktop and mobile preserve the incumbent tokens and layout; no new visual tokens or assets were introduced. The narrow registration finish review returned `ship`; see `.impeccable/review/registration-verdict.md`.
