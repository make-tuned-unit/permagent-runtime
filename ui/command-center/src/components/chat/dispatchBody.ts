/** What a Copy button on an agent message should actually put on the clipboard.
 *
 * When the agent drafts something to be sent on — a prompt for another tool, a
 * message for a person — it almost always arrives as the whole reply wrapped in
 * a fence. The fence is presentation: pasting ``` into the thing you were
 * dispatching to is never what anyone wanted, so it comes off.
 *
 * The rule is deliberately all-or-nothing: the fence is stripped only when it IS
 * the message. The tempting generalisation — "strip the fence whenever there is
 * exactly one" — silently drops the surrounding prose from a reply that explains
 * the draft before giving it, and losing half of what the agent wrote is a much
 * worse failure than leaving three backticks in. So anything with prose around
 * the fence is copied whole, backticks and all, exactly as written.
 *
 * Chrome the agent never wrote — the speaker name, the timestamp, the rendered
 * markdown — is not in `content` in the first place and so cannot leak in.
 */
export function dispatchBody(content: string): string {
  const trimmed = content.trim();
  // Opening fence + optional info string, closing fence at the very end.
  const fenced = /^```[^\n]*\n([\s\S]*?)\n?```$/.exec(trimmed);
  // A body containing its own closing fence means the regex spanned TWO blocks
  // with prose between them; that is the "prose around it" case, copied whole.
  if (fenced && !/^```/m.test(fenced[1])) return fenced[1];
  return trimmed;
}
