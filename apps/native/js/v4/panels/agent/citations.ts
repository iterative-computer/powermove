/** Citation offsets use UTF-16 code units in the rendered plain source message. */
export interface Citation {
  messageId: string;
  text: string;
  start: number;
  end: number;
  comment: string;
}
export function citationLabel(text: string): string {
  const characters = Array.from(text.replace(/\s+/g, " ").trim());
  return characters.length > 32
    ? characters.slice(0, 32).join("") + "…"
    : characters.join("");
}
export function quoteSelection(
  messageId: string,
  source: string,
  start: number,
  end: number,
): Citation | null {
  if (
    !messageId ||
    !Number.isInteger(start) ||
    !Number.isInteger(end) ||
    start < 0 ||
    end <= start ||
    end > source.length
  )
    return null;
  const splitsPair = (at: number) =>
    at > 0 &&
    at < source.length &&
    /[\uD800-\uDBFF]/.test(source[at - 1]!) &&
    /[\uDC00-\uDFFF]/.test(source[at]!);
  if (splitsPair(start) || splitsPair(end)) return null;
  const text = source.slice(start, end);
  return text.trim() ? { messageId, text, start, end, comment: "" } : null;
}

export function isCitation(value: unknown): value is Citation {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const q = value as Record<string, unknown>;
  return (
    typeof q.messageId === "string" &&
    q.messageId.length > 0 &&
    typeof q.text === "string" &&
    q.text.trim().length > 0 &&
    typeof q.comment === "string" &&
    typeof q.start === "number" &&
    Number.isSafeInteger(q.start) &&
    q.start >= 0 &&
    typeof q.end === "number" &&
    Number.isSafeInteger(q.end) &&
    q.end > q.start &&
    q.end - q.start === q.text.length
  );
}
