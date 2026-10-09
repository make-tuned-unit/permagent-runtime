/** The saved Librarian switch is a preference, not evidence of a usable model. */
export type MemoryStatus = 'off' | 'checking' | 'unavailable' | 'needs-ollama' | 'needs-model' | 'installed';

export interface MemoryScheduleChoice { enabled: boolean; model: string }
export interface OllamaInventory { reachable: boolean; installed: Array<{ name: string }> }

export function hasInstalledModel(names: string[], model: string): boolean {
  const chosen = model.trim();
  return !!chosen && names.some(name => name === chosen || name === `${chosen}:latest`);
}

export function memoryStatus({ schedule, ollama, scheduleUnavailable = false, ollamaUnavailable = false }: {
  schedule: MemoryScheduleChoice | null;
  ollama: OllamaInventory | null;
  scheduleUnavailable?: boolean;
  ollamaUnavailable?: boolean;
}): MemoryStatus {
  if (scheduleUnavailable) return 'unavailable';
  if (!schedule) return 'checking';
  if (!schedule.enabled) return 'off';
  if (ollamaUnavailable) return 'unavailable';
  if (!ollama) return 'checking';
  if (!ollama.reachable) return 'needs-ollama';
  return hasInstalledModel(ollama.installed.map(item => item.name), schedule.model)
    ? 'installed' : 'needs-model';
}

export function memoryStatusLabel(status: MemoryStatus): string {
  switch (status) {
    case 'off': return 'Off';
    case 'checking': return 'Checking…';
    case 'unavailable': return 'Status unavailable';
    case 'needs-ollama': return 'Ollama unavailable';
    case 'needs-model': return 'Model missing';
    case 'installed': return 'On · installed';
  }
}
