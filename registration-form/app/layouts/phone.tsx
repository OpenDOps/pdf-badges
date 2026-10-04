import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { StepFrameContext } from "./frame";

export function PhoneLayout({ locale = "ru", children }: { locale?: string; children?: ReactNode }) {
  const { t } = useTranslation();
  return (
    <StepFrameContext.Provider value="phone">
      <main data-testid="frame" className="mx-auto flex w-full max-w-lg flex-col gap-6">
        <h1>{t("frame.phone", { lng: locale })}</h1>
        {children}
      </main>
    </StepFrameContext.Provider>
  );
}
