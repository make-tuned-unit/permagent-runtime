/**
 * Voice replies carry delivery tags (`[pause]`, `[warm]`, …) that the voice
 * server reads to shape speech (`crates/goose-server/src/voice/prosody.rs`).
 * They are saved in the chat message, so every place that SHOWS that text
 * must drop them. Only the known tags are removed, and never a markdown link
 * (`[text](url)`), so ordinary bracketed words survive.
 */
const DELIVERY_TAGS = [
  'pause', 'excited', 'warm', 'calm', 'gentle', 'soft', 'serious', 'thoughtful', 'playful',
];
const TAG_RE = new RegExp(`\\[(?:${DELIVERY_TAGS.join('|')})\\](?!\\()[ \\t]*`, 'gi');

export function stripVoiceDeliveryTags(text: string): string {
  if (!text || text.indexOf('[') === -1) return text;
  return text.replace(TAG_RE, '');
}
