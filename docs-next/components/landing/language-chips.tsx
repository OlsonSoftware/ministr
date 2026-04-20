const LANGUAGES = [
  'Rust',
  'Python',
  'JavaScript',
  'TypeScript',
  'Go',
  'Java',
  'C',
  'C++',
  'Ruby',
  'C#',
  'Swift',
  'Kotlin',
];

export function LanguageChips() {
  return (
    <div className="mx-auto flex max-w-3xl flex-wrap justify-center gap-2">
      {LANGUAGES.map((lang) => (
        <span
          key={lang}
          className="group inline-flex items-center gap-1.5 rounded-full border border-fd-border bg-fd-card px-3 py-1.5 font-mono text-sm font-medium transition hover:-translate-y-px hover:border-[color-mix(in_srgb,var(--color-iris-400)_50%,transparent)] hover:text-[var(--color-iris-500)]"
        >
          <span
            aria-hidden
            className="size-1.5 rounded-full bg-[color-mix(in_srgb,var(--color-iris-400)_40%,transparent)] transition group-hover:bg-[var(--color-iris-500)]"
          />
          {lang}
        </span>
      ))}
    </div>
  );
}
