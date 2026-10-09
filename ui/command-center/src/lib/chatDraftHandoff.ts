import { fileToBase64 } from './api';
import { reconcileChatSetupIntentFinal, type ChatSetupIntentRecord } from './wizardIntent';

export const CHAT_DRAFT_HANDOFF = 'chat_draft_handoff';
export const CHAT_DRAFT_HANDOFF_ACK = 'chat_draft_handoff_ack';
export const CHAT_DRAFT_SYNC = 'chat_draft_sync';
export const CHAT_DRAFT_FINAL = 'chat_draft_final';
export const CHAT_DRAFT_FINAL_ACK = 'chat_draft_final_ack';

export interface EncodedDraftFile { name: string; mime_type: string; data_b64: string }
export interface DraftHandoff { id: string; text: string; files: EncodedDraftFile[]; pendingSetup: ChatSetupIntentRecord | null }
export interface DraftSync { text?: string; files?: EncodedDraftFile[]; source?: string; revision?: number; fileRevision?: number }
export interface DraftFinal { id: string; source: string; revision: number; text: string; files: EncodedDraftFile[]; setup: ChatSetupIntentRecord | null; lastSeenSetupId: string | null }

export async function encodeDraftFiles(files: File[]): Promise<EncodedDraftFile[]> {
  return Promise.all(files.map(async file => ({
    name: file.name, mime_type: file.type, data_b64: await fileToBase64(file),
  })));
}

export function decodeDraftFiles(files: EncodedDraftFile[]): File[] {
  return files.map(file => new File(
    [Uint8Array.from(atob(file.data_b64), char => char.charCodeAt(0))],
    file.name, { type: file.mime_type },
  ));
}

/** Main-window commit. The caller acknowledges only after this returns. */
export function applyFinalDraft(
  payload: DraftFinal,
  setDraft: (draft: { text: string; files: File[] }) => void,
  markReturnPending: () => void,
): void {
  const files = decodeDraftFiles(payload.files);
  setDraft({ text: payload.text, files });
  reconcileChatSetupIntentFinal(payload.setup, payload.lastSeenSetupId);
  markReturnPending();
}

export function isStaleDraftSync(sync: DraftSync, finalRevision: number | undefined): boolean {
  return finalRevision !== undefined && sync.revision !== undefined && sync.revision <= finalRevision;
}
