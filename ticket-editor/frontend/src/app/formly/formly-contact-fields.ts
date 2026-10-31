/** field_<id> keys for the four contact questions, matched by label.
 * The schema is absent on the local catalog route. */
export interface ContactFieldKeys {
  lastNameKey?: string;
  firstNameKey?: string;
  phoneKey?: string;
  emailKey?: string;
}

export interface FormlyFieldLike {
  key?: string | number | (string | number)[];
  type?: string | unknown;
  props?: { label?: string; type?: string };
  validators?: { validation?: string | string[] };
  fieldGroup?: FormlyFieldLike[];
}

interface LeafField {
  key: string;
  label: string;
  type?: string;
  inputType?: string;
  isEmailValidated: boolean;
}

const LAST_NAME_RE = /фамили/i;
const FIRST_NAME_RE = /(?:^|[\s(])имя(?:$|[\s),.:*])/i;
const NON_PERSON_CONTEXT_RE = /компани|организац|должност|отдел|фирм/i;
const PHONE_LABEL_RE = /телефон/i;
const EMAIL_LABEL_RE = /e-?mail|почт/i;

function collectLeafFields(fields: FormlyFieldLike[]): LeafField[] {
  const leaves: LeafField[] = [];

  const visit = (field: FormlyFieldLike) => {
    if (field.key && typeof field.key === 'string') {
      const validation = field.validators?.validation;
      const isEmailValidated = Array.isArray(validation) ? validation.includes('email') : validation === 'email';
      leaves.push({
        key: field.key,
        label: field.props?.label ?? '',
        type: typeof field.type === 'string' ? field.type : undefined,
        inputType: field.props?.type,
        isEmailValidated,
      });
    }
    field.fieldGroup?.forEach(visit);
  };

  fields.forEach(visit);
  return leaves;
}

export function findContactFieldKeys(fields: FormlyFieldLike[]): ContactFieldKeys {
  const leaves = collectLeafFields(fields);

  const phoneKey =
    leaves.find((f) => f.type === 'phone' || f.inputType === 'phone')?.key ??
    leaves.find((f) => PHONE_LABEL_RE.test(f.label))?.key;

  const emailKey =
    leaves.find((f) => f.inputType === 'email' || f.isEmailValidated)?.key ??
    leaves.find((f) => EMAIL_LABEL_RE.test(f.label))?.key;

  const lastNameKey = leaves.find((f) => LAST_NAME_RE.test(f.label))?.key;

  const firstNameKey = leaves.find((f) => FIRST_NAME_RE.test(f.label) && !NON_PERSON_CONTEXT_RE.test(f.label))?.key;

  return { lastNameKey, firstNameKey, phoneKey, emailKey };
}
