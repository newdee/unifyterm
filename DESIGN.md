---
name: UnifyTerm
description: The product page as a travel document, valid in three systems.
colors:
  desk: "#c6cedd"
  paper: "#f2f5fa"
  paper-blue: "#e9effb"
  paper-lilac: "#f0eafa"
  paper-cyan: "#e4f3f4"
  mount: "#ffffff"
  ink: "#151a2e"
  ink-2: "#434a69"
  rule: "#b7c0d5"
  ink-blue: "#2c4bbd"
  ink-purple: "#6a3cb4"
  ink-teal: "#0a6672"
  foil: "#7b4e06"
  panel: "#151a2e"
  panel-ink: "#e8ecf6"
  dark-desk: "#070a14"
  dark-paper: "#141a30"
  dark-ink: "#ece7d6"
  dark-ink-2: "#b5af9a"
  dark-ink-blue: "#7aa2f7"
  dark-ink-purple: "#bb9af7"
  dark-ink-teal: "#7dcfff"
  dark-foil: "#d9b160"
typography:
  titling:
    fontFamily: "Castoro Titling, Songti SC, Noto Serif CJK SC, SimSun, serif"
    fontSize: "clamp(2.4rem, 4.6vw, 3.7rem)"
    fontWeight: 400
    lineHeight: 1
    letterSpacing: "0.1em"
  heading:
    fontFamily: "Castoro Titling, Songti SC, Noto Serif CJK SC, SimSun, serif"
    fontSize: "clamp(1.5rem, 2.7vw, 2.2rem)"
    fontWeight: 400
    lineHeight: 1.14
    letterSpacing: "0.035em"
  statement:
    fontFamily: "B612, PingFang SC, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "clamp(1.35rem, 2.2vw, 1.8rem)"
    fontWeight: 700
    lineHeight: 1.18
    letterSpacing: "-0.012em"
  body:
    fontFamily: "B612, PingFang SC, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "1.0625rem"
    fontWeight: 400
    lineHeight: 1.65
    letterSpacing: "normal"
  label:
    fontFamily: "B612, PingFang SC, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 700
    lineHeight: 1
    letterSpacing: "0.1em"
  code:
    fontFamily: "Overpass Mono, Cascadia Mono, Consolas, monospace"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "normal"
rounded:
  key: "5px"
  control: "6px"
  picture: "4px"
  page: "20px"
  pill: "999px"
spacing:
  page-inset: "clamp(20px, 4.5vw, 60px)"
  page-gap: "clamp(20px, 3vw, 34px)"
  spread-gap: "clamp(28px, 5vw, 66px)"
components:
  page:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    rounded: "{rounded.page}"
    padding: "0 clamp(20px, 4.5vw, 60px) clamp(34px, 5vw, 62px)"
  button-copy:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.paper}"
    rounded: "{rounded.control}"
    padding: "0.62em 1.35em"
  button-copy-hover:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
  tool:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    height: "34px"
    padding: "0 12px"
  command-block:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.panel-ink}"
    rounded: "{rounded.control}"
    padding: "14px 16px"
    typography: "{typography.code}"
---

# Design System: UnifyTerm

## Overview

The page is a travel document. UnifyTerm's claim is that one terminal is valid on Windows, macOS and Linux, and a passport is the object people already know that says "valid in more than one place". Every section is a page of that document, lying on a desk: pale security paper printed with fine line work, a picture of the real program mounted like a holder's photograph, a rubber stamp that carries one measured fact, and a machine-readable band that holds the command which installs it.

The dark theme is the same document seen from its cover: navy, with the titling in gold foil. Nothing else changes between the themes.

Everything on the page is evidence: the pictures are recorded from the program by `tools/promo.ps1`, the numbers on the stamps are measured, and the Observations page lists what is not done. `tools/site-check.ps1` holds the page to the code.

## Colors

### Primary

Ink (`#151a2e` on light, `#ece7d6` on dark) for all running text, on paper (`#f2f5fa`, `#141a30`).

### Secondary

The logo's three strands, as stamp inks: blue, purple, teal. On light paper they are darkened until each passes 4.5:1 (`#2c4bbd`, `#6a3cb4`, `#0a6672`); on dark paper they are the logo's own values (`#7aa2f7`, `#bb9af7`, `#7dcfff`). The line work is drawn in the same three inks at 20% (ground) and 50 to 60% (band).

### Tertiary

Foil (`#7b4e06` light, `#d9b160` dark), from the logo's amber cursor: the seal, the selected tab, focus rings, "Copied". On dark it also colours the titling.

### Neutral

The desk (`#c6cedd`, `#070a14`) is the page background and the top bar. Three tinted papers (`paper-blue`, `paper-lilac`, `paper-cyan`) alternate between sections so that each page of the document is told apart without a border colour.

### Named Rules

- **Ink goes on paper.** A stamp sits on the paper below its picture and bites one corner. Ink over a dark screenshot does not read in either theme.
- **One mask, two themes.** The line work is three black SVG files used as CSS masks over a gradient of the inks. Do not make coloured copies.

## Typography

Castoro Titling sets the name and the section headings: inscriptional capitals, the lettering of a document's cover. B612 sets the text and the field labels; it was drawn for aircraft displays and stays clear at 12 px. Overpass Mono sets commands and field values. Chinese falls back to the system's faces: a Song face under the titling, a Hei face under the text.

### Hierarchy

`titling` (the name, once) → `statement` (the one sentence that says what it is, the only `h1`) → `heading` (one per page) → `body` → `label` (uppercase, tracked, 12 px: field names, tab names, the navigation).

### Named Rules

- **No B612 Mono.** It sets "." and ":" at the left of their cell, and an address reads `https: //raw. github`. Addresses are the most copied text on the page.
- **Chinese field values use the text face.** In a fixed-width face every space between Chinese and Latin takes a full cell.

## Layout

One column of pages, each at most 1180 px wide, 20 to 34 px apart. A feature page is a spread of text (5) and picture (7); every second spread is flipped, and the picture keeps the wide column. Below 860 px a spread is one column, text first. Below 1040 px the navigation moves under the brand and scrolls sideways. The install commands are one column at every width: the longest is 99 letters and must not be cut in two on a desktop.

The data page (name, statement, fields, three entry stamps, the band) fits a 1440 × 900 screen whole.

## Elevation & Depth

Two levels only. A page lies on the desk (`0 22px 44px -26px` shadow and a 1 px light inner edge). A picture is mounted on a page (white mount, 1 px outline, a short shadow). Stamps have no shadow: they are ink, blended with `multiply` on light paper and `screen` on dark.

## Shapes

Pages have 20 px corners (14 px on phones). Pictures and command blocks are nearly square (4 to 6 px). The only pills are the three controls in the top bar. Stamp outlines are the shapes of real entry stamps: rectangle, oval, cut-corner octagon, circle; each is turned a few degrees and passed through one SVG filter (`#ink`) that roughens the edge and breaks the fill.

## Components

### Buttons

`Copy` is the only filled button: ink on paper, inverting on hover. Top-bar tools are 34 px outlined pills.

### Navigation

A sticky bar the colour of the desk: mark and name, seven section links as labels, then language, theme and GitHub.

### Cards / Containers

`.page` is the only container. It always starts with the guilloche band, flush to its edges. Pages are never nested.

### The machine-readable band

The bottom edge of the data page: four tabs (brew, scoop, curl, PowerShell), the chosen command in bold fixed-width letters, `<<<` filler to the right as in a passport's MRZ, and Copy. Windows visitors get scoop first. Copying stamps "COPIED" in foil.

### Stamps

An inline SVG, `currentColor`, with a text alternative that states its fact. On pages below the first screen a stamp lands once when it comes into view (0.28 s, from 1.34× and −7°); with reduced motion, without JavaScript and in print it is simply there.

### Fields

A `dl` of label over value on a dotted rule; labels carry their Chinese name beside the English, as passports do.

## Do's and Don'ts

### Do:

- Put one measured fact on a stamp, and state the same fact in its `aria-label`.
- Use pictures recorded from the program (`tools/promo.ps1 -Stills site/img`), cut to the rows that have something in them.
- Say what is not done on the Observations page when a limit changes.
- Add the Chinese for every new `data-i18n` key; `tools/site-check.ps1` fails without it.

### Don't:

- Don't load anything from another host: fonts, pictures and the video ship with the page.
- Don't add a second accent, a gradient on text, an icon grid or numbered section labels.
- Don't animate anything but the stamps.
- Don't quote users, downloads or stars: there are none to quote.
