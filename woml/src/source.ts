export type WomlDiagnosticPhase = 'parse' | 'validation' | 'compile' | 'runtime';

export interface SourcePosition {
  readonly line: number;
  readonly column: number;
  readonly offset: number;
}

export interface SourceSpan {
  readonly start: SourcePosition;
  readonly end: SourcePosition;
}

export interface WomlDiagnostic {
  readonly code: string;
  readonly phase: WomlDiagnosticPhase;
  readonly message: string;
  readonly file: string;
  readonly location: SourceSpan;
  readonly hint?: string;
}

export interface WomlAdvisoryDiagnostic extends WomlDiagnostic {
  readonly severity: 'warning';
}

export interface WomlSourceAttribute {
  readonly name: string;
  readonly value: string;
  readonly span: SourceSpan;
  readonly nameSpan: SourceSpan;
  readonly valueSpan: SourceSpan;
}

export interface WomlSourceElement {
  readonly kind: 'element';
  readonly name: string;
  readonly attributes: Readonly<Record<string, WomlSourceAttribute>>;
  readonly children: readonly WomlSourceNode[];
  readonly span: SourceSpan;
  readonly openTagSpan: SourceSpan;
}

export interface WomlSourceText {
  readonly kind: 'text';
  readonly value: string;
  readonly span: SourceSpan;
}

export interface WomlSourceRawText {
  readonly kind: 'raw';
  readonly value: string;
  readonly span: SourceSpan;
}

export type WomlSourceNode =
  | WomlSourceElement
  | WomlSourceText
  | WomlSourceRawText;

export interface WomlSourceDocument {
  readonly kind: 'document';
  readonly file: string;
  readonly source: string;
  readonly root: WomlSourceElement;
  readonly span: SourceSpan;
}

export class WomlDiagnosticError extends Error {
  readonly diagnostic: WomlDiagnostic;

  constructor(diagnostic: WomlDiagnostic) {
    super(diagnostic.message);
    this.name = 'WomlDiagnosticError';
    this.diagnostic = diagnostic;
  }
}

export class WomlParseError extends WomlDiagnosticError {
  constructor(diagnostic: WomlDiagnostic) {
    super(diagnostic);
    this.name = 'WomlParseError';
  }
}

export class WomlValidationError extends WomlDiagnosticError {
  constructor(diagnostic: WomlDiagnostic) {
    super(diagnostic);
    this.name = 'WomlValidationError';
  }
}

export class WomlCompileError extends WomlDiagnosticError {
  constructor(diagnostic: WomlDiagnostic) {
    super(diagnostic);
    this.name = 'WomlCompileError';
  }
}

export class SourceFile {
  readonly path: string;
  readonly text: string;
  readonly #lineStarts: readonly number[];

  constructor(path: string, text: string) {
    this.path = path;
    this.text = text;

    const starts = [0];
    for (let index = 0; index < text.length; index += 1) {
      if (text.charCodeAt(index) === 10) {
        starts.push(index + 1);
      }
    }
    this.#lineStarts = starts;
  }

  positionAt(rawOffset: number): SourcePosition {
    const offset = Math.max(0, Math.min(rawOffset, this.text.length));
    let low = 0;
    let high = this.#lineStarts.length;

    while (low + 1 < high) {
      const middle = Math.floor((low + high) / 2);
      if (this.#lineStarts[middle] <= offset) {
        low = middle;
      } else {
        high = middle;
      }
    }

    return {
      line: low + 1,
      column: offset - this.#lineStarts[low] + 1,
      offset,
    };
  }

  offsetAt(line: number, column: number): number {
    const safeLine = Math.max(1, Math.min(line, this.#lineStarts.length));
    const lineStart = this.#lineStarts[safeLine - 1];
    const nextLineStart =
      safeLine < this.#lineStarts.length
        ? this.#lineStarts[safeLine]
        : this.text.length;
    return Math.max(
      lineStart,
      Math.min(lineStart + Math.max(column - 1, 0), nextLineStart),
    );
  }

  span(startOffset: number, endOffset: number): SourceSpan {
    return {
      start: this.positionAt(startOffset),
      end: this.positionAt(endOffset),
    };
  }

  parseError(
    code: string,
    message: string,
    startOffset: number,
    endOffset = startOffset + 1,
    hint?: string,
  ): WomlParseError {
    return new WomlParseError({
      code,
      phase: 'parse',
      message,
      file: this.path,
      location: this.span(startOffset, endOffset),
      ...(hint === undefined ? {} : { hint }),
    });
  }
}

export function isWomlElement(
  node: WomlSourceNode,
): node is WomlSourceElement {
  return node.kind === 'element';
}

export function isWomlRawText(
  node: WomlSourceNode,
): node is WomlSourceRawText {
  return node.kind === 'raw';
}
