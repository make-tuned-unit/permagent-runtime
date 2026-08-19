/**
 * "The agent can't read the attachment I have open" (reported 2026-08-19).
 *
 * Page extraction is `document.body.innerText`. For a PDF or a Word document
 * WKWebView renders a NATIVE viewer, so there is no body text — the extraction
 * returned `''`, and the content bridge turned `''` into "The page appears to
 * be blank or still loading." The agent then reported that it could not read
 * the document, which sounds like a capability failure and is actually a
 * category error: the document is right there, it just is not a DOM.
 *
 * An empty string is the one answer that is never honest for a tab showing a
 * file. These tests pin the honest one, and the capture that makes it useful.
 */
import { describe, it, expect } from 'vitest';
import {
  NON_HTML_STATUS,
  answerForCapture,
  answerForFailedCapture,
  pageContentAnswer,
  type InboxCapture,
  type PageContentResult,
} from './pageContentAnswer';

const pdfTab: PageContentResult = {
  title: 'invoice.pdf',
  url: 'https://example.com/invoice.pdf',
  content:
    'This tab is a application/pdf document, not an HTML page. WebKit renders it in a native ' +
    'viewer, so it has no page text to read.',
  status: NON_HTML_STATUS,
  truncated: false,
  content_type: 'application/pdf',
};

const capture: InboxCapture = {
  filename: 'invoice.pdf',
  path: '/Users/x/.permagent/inbox/invoice.pdf',
  size_bytes: 184320,
  content_type: 'application/pdf',
  url: 'https://example.com/invoice.pdf',
};

describe('a tab that is not an HTML document', () => {
  it('is captured rather than reported as blank', () => {
    const answer = pageContentAnswer(pdfTab);
    expect(answer.capture).toBe(true);
    expect(answer.reply.content).not.toBe('');
    expect(answer.reply.content).not.toContain('blank or still loading');
  });

  /**
   * The exact shipped behaviour, kept here so the difference is visible: the
   * bridge tested emptiness first, so a PDF took the blank-page branch.
   */
  it('the old rule turned it into "blank or still loading"', () => {
    const oldRule = (result: PageContentResult) =>
      !result.content || result.content.trim() === ''
        ? 'The page appears to be blank or still loading.'
        : result.content;
    // What the old extraction actually produced for a PDF tab: nothing.
    expect(oldRule({ ...pdfTab, content: '' })).toBe(
      'The page appears to be blank or still loading.',
    );
  });

  it('answers with the file path once the bytes are on disk', () => {
    const reply = answerForCapture(pdfTab, capture);
    expect(reply.status).toBe(NON_HTML_STATUS);
    expect(reply.content).toContain('/Users/x/.permagent/inbox/invoice.pdf');
    expect(reply.content).toContain('invoice.pdf');
    expect(reply.content).toContain('184320');
  });

  it('says which of the two things failed when the capture does not work', () => {
    const reply = answerForFailedCapture(pdfTab, 'the server answered 403 Forbidden');
    expect(reply.status).toBe(NON_HTML_STATUS);
    expect(reply.content).toContain('not an HTML page');
    expect(reply.content).toContain('403 Forbidden');
    expect(reply.content.trim()).not.toBe('');
  });
});

describe('ordinary pages are untouched', () => {
  it('a real page comes back verbatim, and is never captured', () => {
    const page: PageContentResult = {
      title: 'Docs',
      url: 'https://example.com/docs',
      content: 'Getting started…',
      status: 'ok',
      truncated: false,
    };
    const answer = pageContentAnswer(page);
    expect(answer.capture).toBe(false);
    expect(answer.reply).toBe(page);
  });

  it('an HTML page with no text really is blank or loading — that answer was right', () => {
    const answer = pageContentAnswer({
      title: '',
      url: 'https://example.com/',
      content: '   ',
      status: 'ok',
      truncated: false,
      content_type: 'text/html',
    });
    expect(answer.capture).toBe(false);
    expect(answer.reply.status).toBe('error');
    expect(answer.reply.content).toBe('The page appears to be blank or still loading.');
  });
});
