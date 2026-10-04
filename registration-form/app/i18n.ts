import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import ru from "./locales/ru.json";

void i18n.use(initReactI18next).init({
  lng: "ru",
  fallbackLng: "ru",
  saveMissing: false,
  resources: {
    ru: { translation: ru },
    en: { translation: en },
  },
  interpolation: { escapeValue: false },
  parseMissingKeyHandler(key: string): string {
    throw new Error(`missing i18n key: ${key}`);
  },
});

export default i18n;
