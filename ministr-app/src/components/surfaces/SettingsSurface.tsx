import type { DaemonStatus } from "../../lib/types";
import { GeneralSettings } from "./GeneralSettings";
import { AiAssistantsPanel } from "./AiAssistantsPanel";
import { AboutPanel } from "./AboutPanel";
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

export function SettingsSurface(props: Props) {
  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="max-w-3xl mx-auto space-y-10">
        <section>
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

        <section>
          <H2>AI assistants</H2>
          <div className="mt-4">
            <AiAssistantsPanel
              corpora={props.status.corpora}
              activeCorpusId={props.activeCorpusId}
            />
          </div>
        </section>

        <hr className="border-border" />

        <section>
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
  );
}
