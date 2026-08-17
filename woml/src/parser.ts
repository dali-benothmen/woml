import { XMLParser, XMLValidator } from 'fast-xml-parser';

import { maskRawScriptBodies, type RawScriptBody } from './raw-content';
import {
  SourceFile,
  type SourceSpan,
  type WomlSourceAttribute,
  type WomlSourceDocument,
  type WomlSourceElement,
  type WomlSourceNode,
  type WomlSourceText,
} from './source';

type OrderedNode = Record<string, unknown> & { ':@'?: Record<string, unknown> };

interface ScannedAttribute {
  readonly name: string;
  readonly span: SourceSpan;
  readonly nameSpan: SourceSpan;
  readonly valueSpan: SourceSpan;
}

interface ElementLocation {
  readonly name: string;
  readonly start: number;
  readonly openEnd: number;
  closeStart: number;
  end: number;
  readonly selfClosing: boolean;
  readonly attributes: readonly ScannedAttribute[];
  readonly children: ElementLocation[];
}

export interface ParseWomlOptions {
  readonly file?: string;
}

const parserOptions = {
  preserveOrder: true,
  ignoreAttributes: false,
  attributeNamePrefix: '',
  parseTagValue: false,
  parseAttributeValue: false,
  trimValues: false,
  allowBooleanAttributes: false,
  commentPropName: false,
} as const;

const xmlParser = new XMLParser(parserOptions);

function nodeName(node: OrderedNode): string | undefined {
  return Object.keys(node).find((key) => key !== ':@');
}

function isOrderedElement(node: OrderedNode): boolean {
  const name = nodeName(node);
  return name !== undefined && !name.startsWith('#') && !name.startsWith('?');
}

function validationCode(code: string, message: string): string {
  if (code === 'InvalidAttr') {
    return message.includes('is repeated')
      ? 'WOML_DUPLICATE_ATTRIBUTE'
      : 'WOML_INVALID_ATTRIBUTE';
  }
  return 'WOML_MALFORMED_MARKUP';
}

function validationHint(message: string, scriptCount: number): string | undefined {
  if (scriptCount > 0 && message.includes("closing tag 'script'")) {
    return 'The first literal </script> ends a script body in WOML v0.1. Do not place that exact sequence inside JavaScript text or comments.';
  }
  return undefined;
}

function findTagEnd(source: string, start: number): number {
  let quote: '"' | "'" | undefined;
  for (let index = start + 1; index < source.length; index += 1) {
    const character = source[index];
    if (quote !== undefined) {
      if (character === quote) quote = undefined;
    } else if (character === '"' || character === "'") {
      quote = character;
    } else if (character === '>') {
      return index + 1;
    }
  }
  return source.length;
}

function scanAttributes(
  file: SourceFile,
  tagStart: number,
  tagEnd: number,
  nameEnd: number,
): readonly ScannedAttribute[] {
  const source = file.text;
  const attributes: ScannedAttribute[] = [];
  let cursor = nameEnd;

  while (cursor < tagEnd - 1) {
    while (/\s/.test(source[cursor] ?? '')) cursor += 1;
    if (source[cursor] === '>' || source[cursor] === '/') break;

    const attributeStart = cursor;
    while (/[^\s=/>]/.test(source[cursor] ?? '')) cursor += 1;
    const attributeNameEnd = cursor;
    const name = source.slice(attributeStart, attributeNameEnd);
    while (/\s/.test(source[cursor] ?? '')) cursor += 1;
    if (source[cursor] !== '=') break;
    cursor += 1;
    while (/\s/.test(source[cursor] ?? '')) cursor += 1;
    const quote = source[cursor];
    if (quote !== '"' && quote !== "'") break;
    cursor += 1;
    const valueStart = cursor;
    while (cursor < tagEnd && source[cursor] !== quote) cursor += 1;
    const valueEnd = cursor;
    if (source[cursor] === quote) cursor += 1;

    attributes.push({
      name,
      span: file.span(attributeStart, cursor),
      nameSpan: file.span(attributeStart, attributeNameEnd),
      valueSpan: file.span(valueStart, valueEnd),
    });
  }

  return attributes;
}

function scanElementLocations(
  file: SourceFile,
  maskedSource: string,
): readonly ElementLocation[] {
  const roots: ElementLocation[] = [];
  const stack: ElementLocation[] = [];
  let cursor = 0;

  while (cursor < maskedSource.length) {
    const start = maskedSource.indexOf('<', cursor);
    if (start === -1) break;

    if (maskedSource.startsWith('<!--', start)) {
      const commentEnd = maskedSource.indexOf('-->', start + 4);
      cursor = commentEnd === -1 ? maskedSource.length : commentEnd + 3;
      continue;
    }
    if (maskedSource[start + 1] === '?' || maskedSource[start + 1] === '!') {
      cursor = findTagEnd(maskedSource, start);
      continue;
    }

    const end = findTagEnd(maskedSource, start);
    let nameStart = start + 1;
    const closing = maskedSource[nameStart] === '/';
    if (closing) nameStart += 1;
    while (/\s/.test(maskedSource[nameStart] ?? '')) nameStart += 1;
    let nameEnd = nameStart;
    while (/[A-Za-z0-9:_.-]/.test(maskedSource[nameEnd] ?? '')) nameEnd += 1;
    const name = maskedSource.slice(nameStart, nameEnd);

    if (closing) {
      const location = stack.pop();
      if (location !== undefined) {
        location.closeStart = start;
        location.end = end;
      }
      cursor = end;
      continue;
    }

    const selfClosing = /\/\s*>$/.test(maskedSource.slice(start, end));
    const location: ElementLocation = {
      name,
      start,
      openEnd: end,
      closeStart: end,
      end,
      selfClosing,
      attributes: scanAttributes(file, start, end, nameEnd),
      children: [],
    };

    const parent = stack[stack.length - 1];
    if (parent === undefined) roots.push(location);
    else parent.children.push(location);
    if (!selfClosing) stack.push(location);
    cursor = end;
  }

  return roots;
}

function parsedElementChildren(node: OrderedNode): readonly OrderedNode[] {
  const name = nodeName(node);
  if (name === undefined) return [];
  const value = node[name];
  if (!Array.isArray(value)) return [];
  return value.filter(
    (child): child is OrderedNode =>
      typeof child === 'object' && child !== null && isOrderedElement(child),
  );
}

function stripCommentsFromText(
  file: SourceFile,
  start: number,
  end: number,
): readonly WomlSourceText[] {
  const nodes: WomlSourceText[] = [];
  let cursor = start;

  while (cursor < end) {
    const commentStart = file.text.indexOf('<!--', cursor);
    const segmentEnd = commentStart === -1 || commentStart >= end ? end : commentStart;
    const segment = file.text.slice(cursor, segmentEnd);
    if (segment.trim().length > 0) {
      nodes.push({ kind: 'text', value: segment, span: file.span(cursor, segmentEnd) });
    }
    if (segmentEnd === end) break;
    const commentEnd = file.text.indexOf('-->', commentStart + 4);
    cursor = commentEnd === -1 ? end : Math.min(commentEnd + 3, end);
  }

  return nodes;
}

function buildElement(
  file: SourceFile,
  parsed: OrderedNode,
  location: ElementLocation,
  scriptsByOpeningOffset: ReadonlyMap<number, RawScriptBody>,
): WomlSourceElement {
  const parsedName = nodeName(parsed);
  if (parsedName !== location.name) {
    throw new Error(
      `Internal WOML parser location mismatch: expected ${parsedName}, found ${location.name}.`,
    );
  }

  const parsedAttributes = parsed[':@'] ?? {};
  const attributes: Record<string, WomlSourceAttribute> = {};
  for (const scanned of location.attributes) {
    const value = parsedAttributes[scanned.name];
    attributes[scanned.name] = {
      name: scanned.name,
      value: typeof value === 'string' ? value : String(value ?? ''),
      span: scanned.span,
      nameSpan: scanned.nameSpan,
      valueSpan: scanned.valueSpan,
    };
  }

  const rawScript = scriptsByOpeningOffset.get(location.start);
  const children: WomlSourceNode[] = [];
  if (rawScript !== undefined) {
    children.push({ kind: 'raw', value: rawScript.value, span: rawScript.span });
  } else if (!location.selfClosing) {
    const parsedChildren = parsedElementChildren(parsed);
    let cursor = location.openEnd;

    for (let index = 0; index < location.children.length; index += 1) {
      const childLocation = location.children[index];
      children.push(...stripCommentsFromText(file, cursor, childLocation.start));
      const parsedChild = parsedChildren[index];
      if (parsedChild === undefined) {
        throw new Error(`Internal WOML parser lost child <${childLocation.name}>.`);
      }
      children.push(
        buildElement(file, parsedChild, childLocation, scriptsByOpeningOffset),
      );
      cursor = childLocation.end;
    }
    children.push(...stripCommentsFromText(file, cursor, location.closeStart));
  }

  return {
    kind: 'element',
    name: location.name,
    attributes,
    children,
    span: file.span(location.start, location.end),
    openTagSpan: file.span(location.start, location.openEnd),
  };
}

function firstTextOutsideRoot(
  text: string,
  start: number,
  end: number,
): number | undefined {
  let offset = start;
  while (offset < end) {
    if (/\s/.test(text[offset])) {
      offset += 1;
      continue;
    }
    if (text.startsWith('<!--', offset)) {
      const commentEnd = text.indexOf('-->', offset + 4);
      if (commentEnd === -1 || commentEnd >= end) return undefined;
      offset = commentEnd + 3;
      continue;
    }
    return offset;
  }
  return undefined;
}

export function parseWoml(
  source: string,
  options: ParseWomlOptions = {},
): WomlSourceDocument {
  const file = new SourceFile(options.file ?? '<memory>', source);
  const masked = maskRawScriptBodies(file);

  if (masked.declarationOffsets.length > 0) {
    const offset = masked.declarationOffsets[0];
    throw file.parseError(
      'WOML_DECLARATION_NOT_ALLOWED',
      'WOML does not allow XML declarations, doctypes, CDATA, or processing instructions.',
      offset,
      findTagEnd(source, offset),
    );
  }

  const validation = XMLValidator.validate(masked.source, {
    allowBooleanAttributes: false,
  });
  if (validation !== true) {
    const line = validation.err.line ?? 1;
    const column = validation.err.col ?? 1;
    const offset = file.offsetAt(line, column);
    throw file.parseError(
      validationCode(validation.err.code, validation.err.msg),
      validation.err.msg,
      offset,
      offset + 1,
      validationHint(validation.err.msg, masked.scripts.length),
    );
  }

  let ordered: OrderedNode[];
  try {
    ordered = xmlParser.parse(masked.source) as OrderedNode[];
  } catch (error) {
    throw file.parseError(
      'WOML_MALFORMED_MARKUP',
      error instanceof Error ? error.message : 'Unable to parse WOML markup.',
      0,
    );
  }

  const parsedRoots = ordered.filter(isOrderedElement);
  const locations = scanElementLocations(file, masked.source);
  if (parsedRoots.length !== 1 || locations.length !== 1) {
    const secondRoot = locations[1];
    const offset = secondRoot?.start ?? 0;
    throw file.parseError(
      'WOML_MULTIPLE_ROOTS',
      'A WOML file must contain exactly one root element.',
      offset,
      secondRoot?.openEnd ?? offset + 1,
    );
  }

  const rootLocation = locations[0];
  const leadingOffset = firstTextOutsideRoot(source, 0, rootLocation.start);
  if (leadingOffset !== undefined) {
    throw file.parseError(
      'WOML_TEXT_OUTSIDE_ROOT',
      'Text is not allowed before the WOML root element.',
      leadingOffset,
    );
  }
  const trailingOffset = firstTextOutsideRoot(
    source,
    rootLocation.end,
    source.length,
  );
  if (trailingOffset !== undefined) {
    throw file.parseError(
      'WOML_TEXT_OUTSIDE_ROOT',
      'Text is not allowed after the WOML root element.',
      trailingOffset,
    );
  }

  const scriptsByOpeningOffset = new Map(
    masked.scripts.map((script) => [script.openingTagOffset, script]),
  );
  const root = buildElement(
    file,
    parsedRoots[0],
    rootLocation,
    scriptsByOpeningOffset,
  );

  return {
    kind: 'document',
    file: file.path,
    source,
    root,
    span: file.span(0, source.length),
  };
}
