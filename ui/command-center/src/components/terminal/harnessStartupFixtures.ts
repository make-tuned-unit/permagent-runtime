/**
 * Real PTY startup bytes, captured on this Mac on 2026-08-25.
 *
 * Captured by running each harness under a pseudo-terminal for 12-15 seconds
 * and recording the raw stream. Nothing was typed at them. The base64 is the
 * verbatim byte stream, with one edit: the account name and email that Claude
 * Code prints in its welcome box were replaced with placeholders. No escape
 * sequence was touched, which is the part these fixtures exist to pin down.
 *
 * What they show, and why the readiness gate had to change:
 *
 *   claudeTrustDialog   — `claude` started in a directory it does not yet
 *                         trust. DEC modes set: 25, 2004, 1004, 2031. It turns
 *                         ON bracketed paste at byte 19 and then draws the
 *                         "Quick safety check: Is this a project you created
 *                         or one you trust?" dialog. There is no input box.
 *                         A paste delivered here is answering that dialog.
 *
 *   claudeReadyPrompt   — `claude` started in a directory it trusts. Same four
 *                         modes, and then at byte 61 the alternate screen
 *                         (1049) and at 76-100 mouse tracking (1000, 1002,
 *                         1003, 1006). THAT is the input box.
 *
 *   codexReadyPrompt    — `codex` at its prompt. Bracketed paste at byte 0,
 *                         no alternate screen and no mouse tracking, but
 *                         synchronized output (2026) begin/end pairs from byte
 *                         55 onward as it draws. It redraws in place rather
 *                         than taking the alternate screen.
 *
 * So bracketed paste alone does not mean "there is an input box" — Claude
 * Code's trust dialog sets it too. It means "raw mode". What separates the two
 * captures is whether the harness went on to drive a full-screen surface.
 */

const decode = (b64: string): string => {
  const binary = typeof atob === 'function'
    ? atob(b64)
    : Buffer.from(b64, 'base64').toString('binary');
  return binary;
};

const claudeTrustDialogB64 =
  'GzcbW3IbOBtbPzI1aBtbPzI1bBtbPzIwMDRoG1s/MTAwNGgbWz8yMDMxaA0NChtbMzg7MjsyNTU7MTkzOzdt4pSA4pSA4pSA4pSA' +
  '4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA' +
  '4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA' +
  '4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA' +
  '4pSAG1szOW0NDQobWzJHG1szODsyOzI1NTsxOTM7N20bWzFtQWNjZXNzaW5nG1sxMkd3b3Jrc3BhY2U6G1syMm0bWzM5bQ0NCg0N' +
  'ChtbMkcbWzFtL1VzZXJzL2obWzIybQ0NCg0NChtbMkdRdWljaxtbOEdzYWZldHkbWzE1R2NoZWNrOhtbMjJHSXMbWzI1R3RoaXMb' +
  'WzMwR2EbWzMyR3Byb2plY3QbWzQwR3lvdRtbNDRHY3JlYXRlZBtbNTJHb3IbWzU1R29uZRtbNTlHeW91G1s2M0d0cnVzdD8bWzcw' +
  'RyhMaWtlG1s3Nkd5b3VyDQ0KG1syR293bhtbNkdjb2RlLBtbMTJHYRtbMTRHd2VsbC1rbm93bhtbMjVHb3BlbhtbMzBHc291cmNl' +
  'G1szN0dwcm9qZWN0LBtbNDZHb3IbWzQ5R3dvcmsbWzU0R2Zyb20bWzU5R3lvdXIbWzY0R3RlYW0pLhtbNzFHSWYbWzc0R25vdCwN' +
  'DQobWzJHdGFrZRtbN0dhG1s5R21vbWVudBtbMTZHdG8bWzE5R3JldmlldxtbMjZHd2hhdCdzG1szM0dpbhtbMzZHdGhpcxtbNDFH' +
  'Zm9sZGVyG1s0OEdmaXJzdC4NDQoNDQobWzJHQ2xhdWRlG1s5R0NvZGUnbGwbWzE3R2JlG1syMEdhYmxlG1syNUd0bxtbMjhHcmVh' +
  'ZCwbWzM0R2VkaXQsG1s0MEdhbmQbWzQ0R2V4ZWN1dGUbWzUyR2ZpbGVzG1s1OEdoZXJlLg0NCg0NChtbMkcbWzM4OzI7MTUzOzE1' +
  'MzsxNTNtU2VjdXJpdHkbWzExR2d1aWRlG1szOW0NDQoNDQobWzJHG1szODsyOzE3NzsxODU7MjQ5beKdrxtbNEcbWzM4OzI7MTUz' +
  'OzE1MzsxNTNtMS4bWzdHG1szODsyOzE3NzsxODU7MjQ5bVllcywbWzEyR0kbWzE0R3RydXN0G1syMEd0aGlzG1syNUdmb2xkZXIb' +
  'WzM5bQ0NChtbNEcbWzM4OzI7MTUzOzE1MzsxNTNtMi4bWzdHG1szOW1ObywbWzExR2V4aXQNDQoNDQobWzJHG1szODsyOzE1Mzsx' +
  'NTM7MTUzbUVudGVyG1s4R3RvG1sxMUdjb25maXJtG1sxOUfCtxtbMjFHRXNjG1syNUd0bxtbMjhHY2FuY2VsG1szOW0NDQobWzFD' +
  'G1s0QRtdMTE7PwcbW2MbWz4wcRtbYw==';

const claudeReadyPromptB64 =
  'GzcbW3IbOBtbPzI1aBtbPzI1bBtbPzIwMDRoG1s/MTAwNGgbWz8yMDMxaBtdMTE7PwcbW2MbWz4wcRtbYxtbPzEwNDloG1syShtb' +
  'SBtbPzEwMDBoG1s/MTAwMmgbWz8xMDAzaBtbPzEwMDZoG10wO+KcsyBDbGF1ZGUgQ29kZQcbW0gNG1sxQhtbMzg7MjsyMTU7MTE5' +
  'Ozg3beKVreKUgOKUgOKUgBtbNkdDbGF1ZGUgQ29kZRtbMThHG1szODsyOzE1MzsxNTM7MTUzbXYyLjEuMjQ1G1syN0cbWzM4OzI7' +
  'MjE1OzExOTs4N23ilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDi' +
  'lIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDi' +
  'lIDilIDilIDilIDilIDilIDilIDila4NG1sxQuKUghtbNTRHG1sybeKUghtbNTZHG1syMm0bWzFtVGlwcyBmb3IgZ2V0dGluZyAb' +
  'WzgwRxtbMjJt4pSCDRtbMULilIIbWzE5RxtbMzltG1sxbVdlbGNvbWUgYmFjayBBbGV4IRtbNTRHG1syMm0bWzJtG1szODsyOzIx' +
  'NTsxMTk7ODdt4pSCG1s1NkcbWzIybRtbMW1zdGFydGVkG1s4MEcbWzIybeKUgg0bWzFC4pSCG1s1NEcbWzJt4pSCG1s1NkcbWzM5' +
  'bRtbMjJtUnVuG1s2MEcvaW5pdBtbNjZHdG8bWzY5R2NyZWF0ZRtbNzZHYRtbNzhH4oCmG1s4MEcbWzM4OzI7MjE1OzExOTs4N23i' +
  'lIING1sxQuKUghtbMjRH4paXG1s0ODsyOzIxNTsxMTk7ODdtG1szODsyOzA7MDswbSDilpcgICDilpYgG1s0OW0bWzM4OzI7MjE1' +
  'OzExOTs4N23ilpYbWzU0RxtbMm3ilIIbWzU2RxtbMjJt4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA' +
  '4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSAG1s4MEfilIING1sxQuKUghtbMjVHG1szOW0bWzQ4OzI7MjE1OzExOTs4N20gICAg' +
  'ICAgG1s1NEcbWzQ5bRtbMm0bWzM4OzI7MjE1OzExOTs4N23ilIIbWzU2RxtbMjJtG1sxbVdoYXQncyBuZXcbWzgwRxtbMjJt4pSC' +
  'DRtbMULilIIbWzI2R+KWmOKWmCDilp3ilp0bWzU0RxtbMm3ilIIbWzU2RxtbMzltG1syMm1GaXhlZBtbNjJHYRtbNjRHY3Jhc2gb' +
  'WzcwR29uG1s3M0dzdGFydOKAphtbODBHG1szODsyOzIxNTsxMTk7ODdt4pSCDRtbMULilIIbWzNHG1szODsyOzE1MzsxNTM7MTUz' +
  'bUZhYmxlIDUgwrcgQ2xhdWRlIE1heCDCtyB1c2VyQGV4YW1wbGUuY29tJ3MgG1s1NEcbWzJtG1szODsyOzIxNTsxMTk7ODdt4pSC' +
  'G1s1NkcbWzM5bRtbMjJtQWRkZWQbWzYyR2EbWzY0R0xvb3BzG1s3MEdicmVha2Rvd+KAphtbODBHG1szODsyOzIxNTsxMTk7ODdt' +
  '4pSCDRtbMULilIIbWzNHG1szODsyOzE1MzsxNTM7MTUzbU9yZ2FuaXphdGlvbhtbNTRHG1sybRtbMzg7MjsyMTU7MTE5Ozg3beKU' +
  'ghtbNTZHG1szOW0bWzIybUFkZGVkG1s2MkdgbW9kZWxQaWNrZXJgG1s3NkdzZeKAphtbODBHG1szODsyOzIxNTsxMTk7ODdt4pSC' +
  'DRtbMULilIIbWzExRxtbMzg7MjsxNTM7MTUzOzE1M21+L0RvY3VtZW50cy9kZXYvcGVybWFnZW50LXJ1bnRpbWUbWzU0RxtbMm0b' +
  'WzM4OzI7MjE1OzExOTs4N23ilIIbWzU2RxtbMjJtG1szODsyOzE1MzsxNTM7MTUzbRtbM20vcmVsZWFzZS1ub3RlcyBmb3IgbW9y' +
  'ZRtbODBHG1syM20bWzM4OzI7MjE1OzExOTs4N23ilIING1sxQuKVsOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKVrw0bWzYyQxtbN0IbWzM4OzI7MTUzOzE1' +
  'MzsxNTNt4pePIGhpZ2ggwrcgL2VmZm9ydA0bWzFCG1szODsyOzEzNjsxMzY7MTM2beKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgA0bWzFCG1szOW3i' +
  'na/CoBtbMm1UcnkgImZpeCBsaW50IGVycm9ycyING1sxQhtbMjJtG1szODsyOzEzNjsxMzY7MTM2beKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKU' +
  'gOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgA0b' +
  'WzJDG1sxQhtbMzg7MjsyNTU7MTkzOzdt4pqgIFRyYW5zY3JpcHQgc2F2aW5nIGlzIG9mZiDigJQgaW5oZXJpdGVkIENMQVVERV9D' +
  'T0RFX0NISUxEX1NFU1NJT04gbWFya2VyG1szODsyOzE1MzsxNTM7MTUzbSDCtyBy4oCmDRtbMkMbWzFCG1szODsyOzI1NTsxOTM7' +
  'N23ij7Xij7UgYXV0byBtb2RlIG9uG1szODsyOzE1MzsxNTM7MTUzbSAoc2hpZnQrdGFiIHRvIGN5Y2xlKRtbNjRHG1szODsyOzI1' +
  'NTsxOTM7N20vcmMgY29ubmVjdGluZ+KAphtbMzltG1syNDsxSBtbMjE7M0gbWz8yNWgbWz82bhtbPzI1bBtbSA0bWzFDG1sxM0Ib' +
  'WzM4OzI7MjU1OzE5Mzs3beKaoBtbNEcxIE1DUCBzZXJ2ZXIgbmVlZHMgYXV0aGVudGljYXRpb24bWzM4OzI7MTUzOzE1MzsxNTNt' +
  'IMK3IHJ1biAvbWNwDRtbMkMbWzdCG1szOW0bW0sbWzI0OzFIG1syMTszSBtbPzI1aBtbPzI1bBtbSA0bWzYzQxtbMjNCICAgICAg' +
  'ICAgICAgG1szODsyOzc4OzE4NjsxMDFtL3JjG1szOW0bWzI0OzFIG1syMTszSBtbPzI1aBtbPzI1bBtbSA0bWzJDG1sxOEIbWzM4' +
  'OzI7MjU1OzE5Mzs3bVlvdSd2ZSB1c2VkIDc5JSBvZiB5b3VyIHdlZWtseSBsaW1pdCDCtyByZXNldHMgQXVnIDMxIGF0IDhhbSAo' +
  'QW1lcmljYS9IYWxpZmHigKYbWzM5bRtbMjQ7MUgbWzIxOzNIG1s/MjVo';

const codexReadyPromptB64 =
  'G1s/MjAwNGgbWz40OzBtG1s+N3UbWz8xMDA0aBtbNm4bXTEwOz8bXBtdMTE7PxtcG1s/dRtbYxtbPzIwMjZoG1szOW0bWzQ5bRtb' +
  'MG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0b' +
  'Wz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8y' +
  'NWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwb' +
  'Wz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8y' +
  'MDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2' +
  'bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtb' +
  'PzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bBtbPzIwMjZoG1szOW0bWzQ5bRtbMG0bWz8yNWwbWz8yMDI2bA==';

export const claudeTrustDialogBytes = decode(claudeTrustDialogB64);
export const claudeReadyPromptBytes = decode(claudeReadyPromptB64);
export const codexReadyPromptBytes = decode(codexReadyPromptB64);
