import { useTranslation } from "react-i18next";

import { ScreenFrame } from "../shell/header";

export default function Import() {
  const { t } = useTranslation();
  return <ScreenFrame screen="import" title={t("screen.import")} />;
}
