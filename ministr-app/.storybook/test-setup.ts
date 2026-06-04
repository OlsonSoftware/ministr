import { MotionGlobalConfig } from "motion/react";

/**
 * In the a11y/component test run, jump every animation straight to its final
 * state. framer-motion's `reducedMotion` only skips *transform* animations, not
 * opacity fades — so without this, axe can snapshot text mid-fade-in (opacity
 * 0→1) and report false color-contrast failures. This file is loaded only by
 * the `storybook` Vitest project, so interactive Storybook keeps its motion.
 */
MotionGlobalConfig.skipAnimations = true;
