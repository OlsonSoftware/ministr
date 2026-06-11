/**
 * Brand — identity (DESIGN.md §7): the mark, the wordmark, the amber dot.
 * Identity moments only (window chrome, connect flow) — never a watermark.
 */
export function Brand({ size = "md" }: { size?: "md" | "lg" }) {
  const img = size === "lg" ? "size-8" : "size-5";
  const text = size === "lg" ? "text-2xl" : "text-base";
  return (
    <span className="inline-flex items-center gap-2">
      <img src="/logo.svg" alt="" aria-hidden className={img} />
      <span className={`font-semibold tracking-tight text-ink ${text}`}>
        ministr<span aria-hidden className="text-brand">.</span>
      </span>
    </span>
  );
}
