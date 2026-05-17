import { useEffect, useRef, useState } from "react";
import { useMotionValue, useSpring } from "motion/react";
import { cn } from "../../lib/utils";

interface NumberTickerProps {
  value: number;
  /** Format the animated value for display. Default: rounded + commas. */
  format?: (n: number) => string;
  className?: string;
  /** Flash the text with the accent color when the value changes. */
  flashOnChange?: boolean;
}

const defaultFormat = (n: number) => Math.round(n).toLocaleString();

/**
 * Spring-animated number. Tabular-nums + mono so digits don't jitter.
 * Respects reduced-motion (MotionConfig) — the spring still settles but
 * instantly under reduced motion, which reads as a plain value swap.
 */
export function NumberTicker({
  value,
  format = defaultFormat,
  className,
  flashOnChange = false,
}: NumberTickerProps) {
  const mv = useMotionValue(value);
  const spring = useSpring(mv, { stiffness: 210, damping: 30 });
  const [text, setText] = useState(() => format(value));
  const [flash, setFlash] = useState(false);
  const prev = useRef(value);

  useEffect(() => {
    mv.set(value);
    if (flashOnChange && value !== prev.current) {
      setFlash(true);
      const t = setTimeout(() => setFlash(false), 600);
      prev.current = value;
      return () => clearTimeout(t);
    }
    prev.current = value;
  }, [value, mv, flashOnChange]);

  useEffect(() => {
    const unsub = spring.on("change", (v) => setText(format(v)));
    return () => unsub();
  }, [spring, format]);

  return (
    <span
      className={cn(
        "tabular-nums font-mono transition-colors duration-300",
        flash && "text-accent",
        className,
      )}
    >
      {text}
    </span>
  );
}
