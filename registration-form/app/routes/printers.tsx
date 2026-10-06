import { useTranslation } from "react-i18next";

import { ScreenFrame } from "../shell/header";

export default function Printers() {
  const { t } = useTranslation();
  return <ScreenFrame screen="printers" title={t("screen.printers")} />;
}
