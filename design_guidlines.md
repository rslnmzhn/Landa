# Apple Liquid Glass Design System Guidelines

## Core Principles

- **Optical Depth:** Do not use flat opacity. Use multi-layered refraction with backdrops.
- **Dynamic Materials:** Glass layers must respond to background colors with blur and contrast.
- **Subtlety:** Thin, highly defined borders simulating light catching the edge of a lens.
- **Fluid Viscosity:** Surfaces behave like high-viscosity liquid with surface tension, stretching before detaching and snapping into place.

## Design Tokens (Tailwind / CSS Specifications)

- **Surfaces:**
  - Base Glass: `bg-white/10 dark:bg-white/5`
  - Backdrop Filter: `backdrop-blur-2xl backdrop-saturate-150`
  - Border: `1px solid rgba(255, 255, 255, 0.18)`
  - Inner Glow / Highlight: `inset 0 1px 1px 0 rgba(255, 255, 255, 0.3)`
  - Shadow: `0 20px 50px rgba(0, 0, 0, 0.25)`
  - Border Radius: `rounded-3xl` (24px to 32px for cards, 9999px for pills/tabs)

- **Typography:**
  - Font: SF Pro Display / SF Pro Text (`-apple-system, BlinkMacSystemFont`)
  - Color hierarchy: `text-white/90` (primary), `text-white/60` (secondary), `text-white/40` (tertiary)

- **Controls & Touch Targets:**
  - Minimum touch target: `44x44 pt`
  - Safe Areas: Respect `env(safe-area-inset-top)` and `env(safe-area-inset-bottom)`

## Liquid & Fluid Motion Guidelines

- **Surface Tension & Viscous Morphing (Metaball Effect):**
  - Seamless merging and splitting of elements (e.g., Dynamic Island, detached action pills).
  - SVG Matrix / Contrast Filter for web reproduction:
    ```css
    filter: url('#liquid-filter'); /* feGaussianBlur + feColorMatrix */
    ```
    ```html
    <svg class="hidden">
      <defs>
        <filter id="liquid-filter">
          <feGaussianBlur in="SourceGraphic" stdDeviation="8" result="blur" />
          <feColorMatrix in="blur" mode="matrix" 
            values="1 0 0 0 0  
                    0 1 0 0 0  
                    0 0 1 0 0  
                    0 0 0 19 -9" result="goo" />
          <feComposite in="SourceGraphic" in2="goo" operator="atop" />
        </filter>
      </defs>
    </svg>
    ```

- **Fluid Elasticity & Damping:**
  - Expansion: Elastic stretch along the axis of movement (`scaleX` / `scaleY` distorting up to 1.08x before settling).
  - Spring-driven Viscosity:
    - Quick fluid transitions (pills, switches): `cubic-bezier(0.16, 1, 0.3, 1)` (duration ~240ms)
    - Island expansion & modals: `cubic-bezier(0.25, 1, 0.5, 1)` (duration ~400ms)

- **Interaction Response (Depression):**
  - Press down: Elements shrink slightly (`scale(0.96)`) as if compressed liquid.
  - Release: Over-expansion bounce (`scale(1.02)`) before resting at `1.0`.

## Accessibility & Degradation Rules

- **Prefers Reduced Motion:**
  - Disable gooey/liquid filters, stretching, and distortion.
  - Replace morphing with simple opacity cross-fades (`150ms linear`).
- **Reduce Transparency & High Contrast:**
  - Fallback from liquid glass to solid matte containers (`bg-neutral-900` / `bg-neutral-100`).
  - Set border opacity to solid `1px solid rgba(255, 255, 255, 0.4)`.
