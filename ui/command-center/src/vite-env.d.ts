/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly PERMAGENT_SHORTLIVED_STREAM_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
