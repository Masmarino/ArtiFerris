/** A synchronous translation lookup, for helpers that build user-facing text outside a component. */
export type TranslateFn = (key: string, params?: Record<string, unknown>) => string
