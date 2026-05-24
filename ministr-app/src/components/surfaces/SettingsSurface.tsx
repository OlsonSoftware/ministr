import { useRef, useState, useEffect } from "react";
import { Settings, Bot, Info } from "lucide-react";
import { cn } from "../../lib/utils";
import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { AboutPanel } from "./AboutPanel";
import { AdaptiveSurface } from "../ui/adaptive-surface";
import { H2 } from "../ui/heading";

interface Props {
  status: DaemonStatus;
  activeCorpusId: string | null;
  theme: "system" | "dark" | "light";
  onThemeChange: (t: "system" | "dark" | "light") => void;
  onShowOnboarding: () => void;
  onRefresh: () => void;
  onOpenLogs: () => void;
}

const NAV_ITEMS = [
  { id: "general", label: "General", icon: Settings },
  { id: "ai", label: "AI assistants", icon: Bot },
  { id: "about", label: "About", icon: Info },
] as const;

type SectionId = (typeof NAV_ITEMS)[number]["id"];

export function SettingsSurface(props: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [activeSection, setActiveSection] = useState<SectionId>("general");

  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    const sections = container.querySelectorAll<HTMLElement>("[data-section]");
    if (sections.length === 0) return;

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            const id = entry.target.getAttribute("data-section") as SectionId;
            if (id) setActiveSection(id);
          }
        }
      },
      { root: container, rootMargin: "-20% 0px -60% 0px", threshold: 0 },
    );

    sections.forEach((s) => observer.observe(s));
    return () => observer.disconnect();
  }, []);

  function scrollTo(id: SectionId) {
    const el = scrollRef.current?.querySelector(`[data-section="${id}"]`);
    el?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  return (
    <AdaptiveSurface>
      <div className="h-full flex flex-col @min-[900px]/surface:flex-row min-h-0">
        {/* Sidebar — visible at @md+ */}
        <nav className="hidden @min-[900px]/surface:flex flex-col gap-1 w-[200px] shrink-0 border-r border-border-soft p-4 pt-6">
          {NAV_ITEMS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => scrollTo(id)}
              className={cn(
                "flex items-center gap-2.5 px-3 py-2 rounded-md text-sm font-medium text-left transition-colors duration-150",
                activeSection === id
                  ? "bg-surface-overlay text-text"
                  : "text-text-muted hover:text-text hover:bg-surface-overlay/50",
              )}
            >
              <Icon className="h-4 w-4 shrink-0" strokeWidth={1.8} />
              {label}
            </button>
          ))}
        </nav>

        {/* Content */}
        <div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto p-5">
          <div className="space-y-10 max-w-3xl @min-[900px]/surface:max-w-none">
            <section data-section="general">
              <H2>General</H2>
              <div className="mt-4">
                <GeneralSettings
                  status={props.status}
                  theme={props.theme}
                  onThemeChange={props.onThemeChange}
                  onRefresh={props.onRefresh}
                />
              </div>
            </section>

            <hr className="border-border" />

            <section data-section="ai">
              <H2>AI assistants</H2>
              <div className="mt-4">
                <AiAssistantsPanel
                  corpora={props.status.corpora}
                  activeCorpusId={props.activeCorpusId}
                />
              </div>
            </section>

            <hr className="border-border" />

            <section data-section="about">
              <H2>About</H2>
              <div className="mt-4">
                <AboutPanel
                  status={props.status}
                  onShowOnboarding={props.onShowOnboarding}
                  onRefresh={props.onRefresh}
                  onOpenLogs={props.onOpenLogs}
                />
              </div>
            </section>
          </div>
        </div>
      </div>
    </AdaptiveSurface>
  );
}
