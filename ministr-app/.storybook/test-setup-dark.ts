import { MotionGlobalConfig } from "motion/react";

/**
 * Dark-theme a11y project: render the settled UI (no mid-fade snapshots) AND
 * force the `.dark` class so axe audits every story on the dark surface tier.
 * Paired with the light `storybook` project, this enforces the §9 WCAG floor in
 * BOTH themes mechanically.
 */
MotionGlobalConfig.skipAnimations = true;
document.documentElement.classList.add("dark");
