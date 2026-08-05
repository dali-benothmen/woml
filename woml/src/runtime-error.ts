export type WorkflowExecutionErrorCode =
  | 'WOML_INVALID_COMPILED_MODEL'
  | 'WOML_INVALID_DAG'
  | 'WOML_INVALID_TRIGGER'
  | 'WOML_UNSUPPORTED_EDGE_CONDITION'
  | 'WOML_UNSUPPORTED_INPUT_EXPRESSION'
  | 'WOML_UNKNOWN_HANDLER'
  | 'WOML_INVALID_HANDLER_INPUT'
  | 'WOML_HANDLER_FAILED'
  | 'WOML_SCRIPT_FAILED'
  | 'WOML_NON_JSON_RESULT';

export interface WorkflowExecutionErrorOptions {
  readonly nodeId?: string;
  readonly cause?: unknown;
  readonly remoteStack?: string;
}

export class WorkflowExecutionError extends Error {
  readonly code: WorkflowExecutionErrorCode;
  readonly nodeId?: string;
  readonly remoteStack?: string;

  constructor(
    code: WorkflowExecutionErrorCode,
    message: string,
    options: WorkflowExecutionErrorOptions = {},
  ) {
    super(message, options.cause === undefined ? undefined : { cause: options.cause });
    this.name = 'WorkflowExecutionError';
    this.code = code;
    if (options.nodeId !== undefined) this.nodeId = options.nodeId;
    if (options.remoteStack !== undefined) this.remoteStack = options.remoteStack;
  }
}
