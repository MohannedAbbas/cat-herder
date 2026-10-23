// Runtime configuration sourced from Vite environment variables (see .env / VITE_*).
// The mock data layer is the default until the control-plane REST endpoints exist.
//
// apiBaseUrl defaults to '' (relative), which works both in dev (Vite proxies /v1)
// and in-cluster (the Ingress routes /v1 to the API Service, same-origin).
export const config = {
  apiBaseUrl: import.meta.env.VITE_API_BASE_URL ?? '',
  useMock: import.meta.env.VITE_USE_MOCK !== 'false',
} as const
