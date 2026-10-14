/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_API_BASE_URL?: string;
  readonly VITE_BUILD_NUMBER?: string;
  readonly VITE_GIT_SHA?: string;
  readonly PERMAGENT_SHORTLIVED_STREAM_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
