export const PRODUCT_NAME = "permagent";
export const PRODUCT_LABEL = "Permagent";

/** Rewrite upstream Goose copy so it never appears in the product UI. */
export function brandCopy(text: string): string {
  return text
    .replace(/`goose configure`/g, "`permagent configure`")
    .replace(/\bGoose\b/g, PRODUCT_LABEL)
    .replace(/\bgoose\b/g, PRODUCT_NAME);
}
