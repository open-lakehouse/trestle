/** The message of a caught value: an `Error`'s message, else the value as text. */
export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
