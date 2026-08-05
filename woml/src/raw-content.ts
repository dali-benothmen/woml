import { SourceFile, type SourceSpan } from './source';

export interface RawScriptBody {
  readonly index: number;
  readonly openingTagOffset: number;
  readonly value: string;
  readonly span: SourceSpan;
}

export interface MaskedWomlSource {
  readonly source: string;
  readonly scripts: readonly RawScriptBody[];
  readonly declarationOffsets: readonly number[];
}

interface MarkupToken {
  readonly start: number;
  readonly end: number;
  readonly name?: string;
  readonly closing: boolean;
  readonly selfClosing: boolean;
  readonly declaration: boolean;
}

function findMarkupTokenEnd(source: string, start: number): number {
  let quote: '"' | "'" | undefined;

  for (let index = start + 1; index < source.length; index += 1) {
    const character = source[index];
    if (quote !== undefined) {
      if (character === quote) quote = undefined;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (character === '>') return index + 1;
  }

  return source.length;
}

function readMarkupToken(source: string, start: number): MarkupToken {
  if (source.startsWith('<!--', start)) {
    const commentEnd = source.indexOf('-->', start + 4);
    return {
      start,
      end: commentEnd === -1 ? source.length : commentEnd + 3,
      closing: false,
      selfClosing: false,
      declaration: false,
    };
  }

  const end = findMarkupTokenEnd(source, start);
  let cursor = start + 1;
  const declaration = source[cursor] === '?' || source[cursor] === '!';
  const closing = source[cursor] === '/';
  if (closing) cursor += 1;
  while (/\s/.test(source[cursor] ?? '')) cursor += 1;

  const nameStart = cursor;
  while (/[A-Za-z0-9:_-]/.test(source[cursor] ?? '')) cursor += 1;
  const name = cursor === nameStart ? undefined : source.slice(nameStart, cursor);
  const tokenText = source.slice(start, end);

  return {
    start,
    end,
    name,
    closing,
    selfClosing: /\/\s*>$/.test(tokenText),
    declaration,
  };
}

function maskBody(body: string): string {
  return body.replace(/[^\r\n\t]/g, ' ');
}

export function maskRawScriptBodies(file: SourceFile): MaskedWomlSource {
  const source = file.text;
  const maskedParts: string[] = [];
  const scripts: RawScriptBody[] = [];
  const declarationOffsets: number[] = [];
  let cursor = 0;
  let emittedUntil = 0;

  while (cursor < source.length) {
    const tagStart = source.indexOf('<', cursor);
    if (tagStart === -1) break;

    const token = readMarkupToken(source, tagStart);
    if (token.declaration) declarationOffsets.push(token.start);

    if (
      token.name === 'script' &&
      !token.closing &&
      !token.selfClosing &&
      !token.declaration
    ) {
      const bodyStart = token.end;
      const closingTagStart = source.indexOf('</script>', bodyStart);
      if (closingTagStart === -1) {
        throw file.parseError(
          'WOML_UNCLOSED_RAW_BODY',
          'The <script> body is not closed by the required literal </script> terminator.',
          token.start,
          token.end,
          'Add </script>. The terminator is case-sensitive in WOML v0.1.',
        );
      }

      const body = source.slice(bodyStart, closingTagStart);
      maskedParts.push(source.slice(emittedUntil, bodyStart), maskBody(body));
      scripts.push({
        index: scripts.length,
        openingTagOffset: token.start,
        value: body,
        span: file.span(bodyStart, closingTagStart),
      });
      emittedUntil = closingTagStart;
      cursor = closingTagStart + '</script>'.length;
      continue;
    }

    cursor = Math.max(token.end, tagStart + 1);
  }

  maskedParts.push(source.slice(emittedUntil));
  const maskedSource = maskedParts.join('');
  if (maskedSource.length !== source.length) {
    throw new Error('Internal WOML raw-content masking changed source length.');
  }

  return {
    source: maskedSource,
    scripts,
    declarationOffsets,
  };
}
