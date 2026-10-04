import type { ReactNode } from "react";

import { StepFrameContext, useKioskFrame } from "./frame";

export function KioskLayout({ children }: { children?: ReactNode }) {
  const frame = useKioskFrame();
  return (
    <StepFrameContext.Provider value={frame}>
      <main
        data-testid="frame"
        className={`mx-auto flex w-full flex-col gap-6 ${frame === "web" ? "max-w-web" : "max-w-kiosk"}`}
      >
        {children}
      </main>
    </StepFrameContext.Provider>
  );
}
