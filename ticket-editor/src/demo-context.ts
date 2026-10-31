/** Demo guest for the palette and the canvas. The editor reads this object. It does not fetch it. */

export type DemoFieldKind =
  | 'text'
  | 'textarea'
  | 'choice'
  | 'multichoice'
  | 'boolean'
  | 'phone'
  | 'email'
  | 'date';

export type DemoSample = string | string[] | boolean;

export interface DemoField {
  key: string;
  label: string;
  kind: DemoFieldKind;
  section: string;
  sample: DemoSample;
}

export interface DemoContext {
  fields: DemoField[];
  ticket: { number: string; code: string; ticket_type_name: string };
  form_data: Record<string, DemoSample>;
  has_vcard: boolean;
  codes: {
    barcode: string;
    qrcode: string;
    vcard_qrcode: string;
    barcode_svg: string;
    qrcode_svg: string;
    vcard_qrcode_svg: string;
  };
  geometry: { ean13_modules: number; qr_modules: number };
}

export const demoContext: DemoContext = {
  fields: [
    { key: 'field_1', label: 'Фамилия', kind: 'text', section: 'Гость', sample: 'Петрова' },
    { key: 'field_2', label: 'Имя', kind: 'text', section: 'Гость', sample: 'Анна' },
    { key: 'field_3', label: 'Компания', kind: 'text', section: 'Гость', sample: 'ООО «Ромашка-Экспо»' },
    { key: 'field_4', label: 'Должность', kind: 'text', section: 'Гость', sample: 'Руководитель отдела маркетинга' },
    { key: 'field_5', label: 'Теги', kind: 'multichoice', section: 'Гость', sample: ['Доклад', 'Стенд'] },
    { key: 'field_6', label: 'Спикер', kind: 'boolean', section: 'Гость', sample: true },
  ],
  ticket: { number: 'A-00042', code: '2000000000008', ticket_type_name: 'Standard' },
  form_data: {
    field_1: 'Петрова',
    field_2: 'Анна',
    field_3: 'ООО «Ромашка-Экспо»',
    field_4: 'Руководитель отдела маркетинга',
    field_5: ['Доклад', 'Стенд'],
    field_6: true,
  },
  has_vcard: false,
  codes: {
    barcode: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAOoAAABQCAIAAACYp34KAAAEJElEQVR4nO2SQYoDURBC+/6XnsBs5QVj4kIoV4LaSv96/r7G8w/l6vneqalH8Kmz3a6cNuw6SfVTGX7xCWPW+wf2nfSznO+/f5Jeu3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9luPOttyunDbtOUv1Uhjvferty2rDrJNVPZbjzrbcrpw27TlL9VIY733q7ctqw6yTVT2W48623K6cNu05S/VSGO996u3LasOsk1U9leAHVAQfyrfT+vAAAAABJRU5ErkJggg==",
    qrcode: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAHQAAAB0CAYAAABUmhYnAAAAAklEQVR4AewaftIAAAK3SURBVO3BQW7jUAwFwX6E7n/lHi+5+oBg2ZMwrIovrDGKNUqxRinWKMUapVijFGuUYo1SrFGKNUqxRinWKMUapVijFGuUYo1y8aYkfJPKSRI6lS4JJypdEr5J5R3FGqVYoxRrlIuHqTwpCXeo3KFyh8qTkvCkYo1SrFGKNcrFhyXhDpV3JOGOJHQqdyThDpVPKtYoxRqlWKNcDKfylxRrlGKNUqxRLv6YJJyo/GbFGqVYoxRrlIsPU/mkJNyh8iSVn6RYoxRrlGKNcvGwJPxPKl0SOpUuCZ3KSRJ+smKNUqxRijVKfGGwJHQqkxVrlGKNUqxRLt6UhE7lJAmdSpeEE5UuCZ1Kp9Il4Q6VLgmdyk9SrFGKNUqxRrn4sCScJKFT+SaVLgnvSEKn0iWhU3lSsUYp1ijFGuXiTSonKl0SOpUuCScqJ0noVO5Q6ZJwkoROpUvCNxVrlGKNUqxRLh6WhBOVO1S6JHQqXRJOVLokfJJKl4QuCZ3KO4o1SrFGKdYo8YX/KAmdSpeEO1ROktCpdEm4Q+UkCScqTyrWKMUapVijxBd+sSTcoXKShCepfFOxRinWKMUaJb7whiR8k8odSbhDpUtCp9Il4R0qTyrWKMUapVijXDxM5UlJOEnCicqTktCpdEnoVE6S0Km8o1ijFGuUYo1y8WFJuEPlDpUuCV0SOpUuCXeonKh0SfimYo1SrFGKNcrFL5eETuUOlS4JJ0k4UelUuiR8UrFGKdYoxRrl4pdT6ZJwh8odKl0STpLQqXRJeFKxRinWKMUa5eLDVL5J5SQJXRI6lZMknCShU+mS0Kk8qVijFGuUYo0SX3hDEr5JpUvCicodSehUTpJwh8onFWuUYo1SrFHiC2uMYo1SrFGKNUqxRinWKMUapVijFGuUYo1SrFGKNUqxRinWKMUapVij/AMlSBLpirJcHgAAAABJRU5ErkJggg==",
    vcard_qrcode: '',
    barcode_svg: "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMTcgNjAiIHByZXNlcnZlQXNwZWN0UmF0aW89Im5vbmUiIHNoYXBlLXJlbmRlcmluZz0iY3Jpc3BFZGdlcyIgZmlsbD0iIzAwMCI+PHJlY3QgeD0iMTEiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMTMiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMTciIHk9IjAiIHdpZHRoPSIyIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMjAiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMjQiIHk9IjAiIHdpZHRoPSIyIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMjciIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMjkiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMzIiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMzYiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMzkiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNDUiIHk9IjAiIHdpZHRoPSIyIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNDgiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNTAiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNTMiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNTciIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNTkiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNjEiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNjYiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNjgiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNzMiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iNzUiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iODAiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iODIiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iODciIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iODkiIHk9IjAiIHdpZHRoPSIzIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iOTQiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iOTYiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iOTkiIHk9IjAiIHdpZHRoPSIxIiBoZWlnaHQ9IjYwIi8+PHJlY3QgeD0iMTAzIiB5PSIwIiB3aWR0aD0iMSIgaGVpZ2h0PSI2MCIvPjxyZWN0IHg9IjEwNSIgeT0iMCIgd2lkdGg9IjEiIGhlaWdodD0iNjAiLz48L3N2Zz4=",
    qrcode_svg: "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyOSAyOSIgc2hhcGUtcmVuZGVyaW5nPSJjcmlzcEVkZ2VzIj48cGF0aCBmaWxsPSIjZmZmZmZmIiBkPSJNMCAwaDI5djI5SDB6Ii8+PHBhdGggc3Ryb2tlPSIjMDAwMDAwIiBkPSJNNCA0LjVoN20yIDBoMW0xIDBoMm0xIDBoN000IDUuNWgxbTUgMGgxbTQgMGgxbTIgMGgxbTUgMGgxTTQgNi41aDFtMSAwaDNtMSAwaDFtMiAwaDRtMSAwaDFtMSAwaDNtMSAwaDFNNCA3LjVoMW0xIDBoM20xIDBoMW0yIDBoMW00IDBoMW0xIDBoM20xIDBoMU00IDguNWgxbTEgMGgzbTEgMGgxbTIgMGgxbTEgMGgybTEgMGgxbTEgMGgzbTEgMGgxTTQgOS41aDFtNSAwaDFtMSAwaDJtNCAwaDFtNSAwaDFNNCAxMC41aDdtMSAwaDFtMSAwaDFtMSAwaDFtMSAwaDdNMTQgMTEuNWgxTTQgMTIuNWgxbTIgMGgxbTEgMGgybTEgMGgxbTIgMGgzbTEgMGgxTTQgMTMuNWgxbTEgMGgzbTMgMGgxbTIgMGgxbTEgMGgxbTEgMGgxbTEgMGgxbTEgMGgxTTcgMTQuNWgxbTEgMGgybTEgMGgybTEgMGgxbTIgMGgzbTEgMGgyTTUgMTUuNWgybTIgMGgxbTEgMGgxbTEgMGgxbTIgMGg1bTIgMGgyTTggMTYuNWgxbTEgMGgzbTIgMGgxbTEgMGgzbTIgMGgyTTEyIDE3LjVoM20yIDBoNU00IDE4LjVoN20zIDBoM20xIDBoMW0xIDBoNE00IDE5LjVoMW01IDBoMW0xIDBoMm00IDBoMm0xIDBoMW0yIDBoMU00IDIwLjVoMW0xIDBoM20xIDBoMW0yIDBoMm0xIDBoM20zIDBoM000IDIxLjVoMW0xIDBoM20xIDBoMW0xIDBoMW00IDBoNG0yIDBoMk00IDIyLjVoMW0xIDBoM20xIDBoMW0yIDBoM20xIDBoMm0xIDBoMW0xIDBoMW0xIDBoMU00IDIzLjVoMW01IDBoMW00IDBoMm0yIDBoM20xIDBoMU00IDI0LjVoN20xIDBoMm0zIDBoMW0yIDBoMyIvPjwvc3ZnPgo=",
    vcard_qrcode_svg: '',
  },
  geometry: { ean13_modules: 117, qr_modules: 29 },
};
