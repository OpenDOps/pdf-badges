import { useTranslation } from "react-i18next";

import { ScreenFrame } from "../shell/header";

export default function Print() {
  const { t } = useTranslation();
  return <ScreenFrame screen="print" title={t("screen.print")} />;
}
