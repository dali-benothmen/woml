import type { SecretReferenceExpression } from './model';
import type { WomlSourceAttribute, WomlSourceDocument } from './source';
import { WomlValidationError } from './source';

export const WOML_SECRET_NAME_PATTERN = /^[A-Z][A-Z0-9_]*$/;

const secretReferencePattern = /^\{\{secrets\.([A-Z][A-Z0-9_]*)\}\}$/;

export function isValidSecretName(name: string): boolean {
  return WOML_SECRET_NAME_PATTERN.test(name);
}

export function parseSecretReference(
  value: string
): SecretReferenceExpression | undefined {
  const match = secretReferencePattern.exec(value);
  if (match === null) return undefined;
  return { kind: 'secretReference', name: match[1] };
}

export function requireSecretReference(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): SecretReferenceExpression {
  const reference = parseSecretReference(attribute.value);
  if (reference !== undefined) return reference;

  const looksLikeSecretReference = attribute.value.includes('{{secrets.');
  throw new WomlValidationError({
    code: looksLikeSecretReference
      ? 'WOML_SECRET_REFERENCE_INVALID'
      : 'WOML_SECRET_LITERAL_FORBIDDEN',
    phase: 'validation',
    message: looksLikeSecretReference
      ? 'Secret references must use the exact form {{secrets.NAME}}, where NAME starts with A-Z and contains only A-Z, 0-9, or underscore.'
      : `Attribute "${attribute.name}" accepts a secret reference, never a literal credential.`,
    file: document.file,
    location: attribute.valueSpan,
    hint: `Use ${attribute.name}="{{secrets.SECRET_NAME}}" and store the value with woml secrets set SECRET_NAME.`,
  });
}
