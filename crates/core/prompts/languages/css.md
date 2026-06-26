# CSS Starter Rules

Report only when the diff clearly hits one of these language traps and creates real risk:

- [CSS1] Using `!important` to forcibly override styles, creating an unbeatable specificity deadlock instead of fixing the selector structure.
- [CSS2] Hardcoded or magic `z-index` (e.g. `9999`) with no layering system, causing runaway stacking contexts and occlusion bugs.
- [CSS3] Over-reliance on selector specificity (deep nesting, ID stacking) fighting each other, making the style source hard to predict.
- [CSS4] Fixed units / missing overflow handling causing content to be clipped, horizontal scrolling, or broken layout across viewports (consider `min/max`, `overflow`, relative units).
- [CSS5] Hardcoded color/size literals bypassing existing design variables (CSS custom properties / theme tokens), breaking theme and dark-mode consistency.
