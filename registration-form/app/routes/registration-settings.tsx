import { useTranslation } from "react-i18next";

import { ScreenFrame } from "../shell/header";

export default function RegistrationSettings() {
  const { t } = useTranslation();
  return <ScreenFrame screen="registration" title={t("screen.registration")} />;
}
