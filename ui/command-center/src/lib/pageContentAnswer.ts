// What to tell the agent when it asks to read the browser tab.
//
// Reported 2026-08-19: "the agent can't read an attachment I have open".
//
// Page extraction is `document.body.innerText`. For a PDF or a Word document
// WKWebView renders a NATIVE viewer — there is no body text, so the extraction
// returned `''`, and this bridge turned `''` into "The page appears to be blank
// or still loading." The agent then said it could not read the document, which
// is true but for entirely the wrong reason: the document is right there, it is
// simply not a DOM. An empty string is the one answer that is never honest for
// a tab showing a file.
//
// The desktop side now says so (`status: 'non_html_document'`, see browser.rs
// `classify_page_content`). This decides what to do about it: capture the file
// into the inbox and answer with the PATH, because Permagent can already read a
// file — the Reader and the local OCR — and can do nothing whatever with a
// native viewer's empty body.
//
// Pure, so the decision is testable without a webview, a daemon or a PDF.

/** Mirrors `browser.rs`'s `NON_HTML_STATUS`. Renaming one side breaks both. */
export const NON_HTML_STATUS = 'non_html_document';

export interface PageContentResult {
  title: string;
  url: string;
  content: string;
  status: string;
  truncated: boolean;
  content_type?: string | null;
}

/** What `save_tab_to_inbox` returns on success. */
export interface InboxCapture {
  filename: string;
  path: string;
  size_bytes: number;
  content_type?: string | null;
  url: string;
}

export interface PageContentAnswer {
  /** Ask the desktop to capture this tab into the inbox before answering. */
  capture: boolean;
  /** What to send back if the capture is not attempted, or fails. */
  reply: PageContentResult;
}

/**
 * Decide the answer for a raw extraction result.
 *
 * - a non-HTML tab is worth capturing, and its honest description is the
 *   fallback if the capture fails;
 * - an HTML tab with no text really is blank or still loading, and that answer
 *   was right all along;
 * - anything else is just the page.
 */
export function pageContentAnswer(result: PageContentResult): PageContentAnswer {
  if (result.status === NON_HTML_STATUS) {
    return { capture: true, reply: result };
  }
  if (!result.content || result.content.trim() === '') {
    return {
      capture: false,
      reply: {
        title: result.title,
        url: result.url,
        content: 'The page appears to be blank or still loading.',
        status: 'error',
        truncated: false,
      },
    };
  }
  return { capture: false, reply: result };
}

/**
 * The answer once the file is on disk: say where it is, in the imperative the
 * agent needs. The point of the whole path is that "read this attachment"
 * becomes "read this file".
 */
export function answerForCapture(
  result: PageContentResult,
  capture: InboxCapture,
): PageContentResult {
  return {
    title: result.title || capture.filename,
    url: result.url || capture.url,
    content:
      `This tab is a ${capture.content_type || 'non-HTML'} document, not an HTML page, so it has ` +
      `no page text. It has been saved to the Permagent inbox as "${capture.filename}" ` +
      `(${capture.size_bytes} bytes). Read the file directly at: ${capture.path}`,
    status: NON_HTML_STATUS,
    truncated: false,
    content_type: capture.content_type ?? result.content_type ?? null,
  };
}

/**
 * The answer when the capture failed. Still honest, still not an empty string,
 * and it says which of the two things went wrong — the tab is not a DOM, AND
 * the file could not be fetched.
 */
export function answerForFailedCapture(
  result: PageContentResult,
  error: string,
): PageContentResult {
  return {
    ...result,
    content: `${result.content} Saving it to the inbox failed: ${error}`,
    status: NON_HTML_STATUS,
  };
}
