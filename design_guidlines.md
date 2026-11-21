# Apple Liquid Glass Design System Guidelines

## Core Principles
- Optical Depth: Do not use flat opacity. Use multi-layered refraction with backdrops.
- Dynamic Materials: Glass layers must respond to background colors with blur and contrast.
- Subtlety: Thin, highly defined borders simulating light catching the edge of a lens.

## Design Tokens (Tailwind / CSS Specifications)
- Surfaces:
  - Base Glass: bg-white/10 dark:bg-white/5
  - Backdrop Filter: backdrop-blur-2xl backdrop-saturate-150
  - Border: 1px solid rgba(255, 255, 255, 0.18)
  - Inner Glow / Highlight: inset 0 1px 1px 0 rgba(255, 255, 255, 0.3)
  - Shadow: 0 20px 50px rgba(0, 0, 0, 0.25)
  - Border Radius: rounded-3xl (24px to 32px for cards, 9999px for pills/tabs)
- Typography:
  - Font: SF Pro Display / SF Pro Text (-apple-system, BlinkMacSystemFont)
  - Color hierarchy: text-white/90 (primary), text-white/60 (secondary), text-white/40 (tertiary)
- Controls & Touch Targets:
  - Minimum touch target: 44x44 pt
  - Safe Areas: Respect env(safe-area-inset-top) and env(safe-area-inset-bottom)
