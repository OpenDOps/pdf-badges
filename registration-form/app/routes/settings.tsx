import { useTranslation } from "react-i18next";

import { ScreenFrame, ScreenLink } from "../shell/header";

export default function Settings() {
  const { t } = useTranslation();
  return (
    <ScreenFrame screen="settings" title={t("screen.settings")}>
      <ScreenLink to="/printers">{t("settings.printers")}</ScreenLink>
      <ScreenLink to="/settings/registration">{t("settings.registration")}</ScreenLink>
    </ScreenFrame>
  );
}
