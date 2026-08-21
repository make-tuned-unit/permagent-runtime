import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  applyAtMention,
  atQuery,
  formatShellPrompt,
  formatTranscript,
  fuzzyFiles,
  lastAssistantText,
  parseBang,
  scoreFile,
  stripAtQuery,
} from "./editorExtras.js";

describe("atQuery", () => {
  it("opens on a trailing @token", () => {
    assert.deepEqual(atQuery("look at @src/t"), {
      start: 8,
      query: "src/t",
    });
    assert.deepEqual(atQuery("@"), { start: 0, query: "" });
  });

  it("stays closed for slash, bang, and finished mentions", () => {
    assert.equal(atQuery("/help"), null);
    assert.equal(atQuery("!ls"), null);
    assert.equal(atQuery("see @foo bar"), null);
  });
});

describe("applyAtMention / stripAtQuery", () => {
  it("replaces the query with a path", () => {
    assert.equal(applyAtMention("see @tui", 4, "ui/text/src/tui.tsx"), "see @ui/text/src/tui.tsx ");
  });

  it("strips an incomplete mention", () => {
    assert.equal(stripAtQuery("see @tui"), "see");
  });
});

describe("fuzzyFiles", () => {
  it("ranks basename matches first", () => {
    const files = ["ui/text/src/tui.tsx", "crates/goose/src/lib.rs", "README.md"];
    assert.equal(fuzzyFiles(files, "tui")[0], "ui/text/src/tui.tsx");
    assert.ok(scoreFile("README.md", "xyz") === 0);
  });
});

describe("parseBang", () => {
  it("splits ! and !!", () => {
    assert.deepEqual(parseBang("! git status"), {
      sendToModel: true,
      command: "git status",
    });
    assert.deepEqual(parseBang("!!ls"), {
      sendToModel: false,
      command: "ls",
    });
    assert.equal(parseBang("hello"), null);
  });
});

describe("transcript helpers", () => {
  it("pulls the last assistant text", () => {
    assert.equal(
      lastAssistantText([
        {
          responseItems: [
            { itemType: "content_chunk", content: { type: "text", text: "one" } },
          ],
        },
        {
          responseItems: [
            { itemType: "content_chunk", content: { type: "text", text: "two" } },
          ],
        },
      ]),
      "two",
    );
  });

  it("formats a markdown transcript", () => {
    const md = formatTranscript([
      {
        userText: "hi",
        responseItems: [
          { itemType: "content_chunk", content: { type: "text", text: "hello" } },
        ],
      },
    ]);
    assert.match(md, /## User/);
    assert.match(md, /hello/);
  });

  it("formats a shell prompt block", () => {
    assert.match(formatShellPrompt("ls", "a\nb", true), /^\$ ls/);
    assert.match(formatShellPrompt("false", "err", false), /failed/);
  });
});
