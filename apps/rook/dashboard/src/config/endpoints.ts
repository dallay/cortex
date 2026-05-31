/**
 * Available API endpoints in Rook.
 * These are the paths clients can use to connect to Rook.
 *
 * Based on OpenAI-compatible and Anthropic-compatible standards.
 */
export interface Endpoint {
  path: string
  method: 'GET' | 'POST'
  descriptionKey: string
  category: 'core' | 'media' | 'utility'
}

export const endpointsConfig = {
  core: [
    {
      path: '/v1/chat/completions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.chatCompletions',
    },
    {
      path: '/v1/responses',
      method: 'POST' as const,
      descriptionKey: 'endpoints.responses',
    },
    {
      path: '/v1/completions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.completions',
    },
    {
      path: '/v1/messages',
      method: 'POST' as const,
      descriptionKey: 'endpoints.messages',
    },
  ],
  media: [
    {
      path: '/v1/embeddings',
      method: 'POST' as const,
      descriptionKey: 'endpoints.embeddings',
    },
    {
      path: '/v1/images/generations',
      method: 'POST' as const,
      descriptionKey: 'endpoints.imagesGenerations',
    },
    {
      path: '/v1/audio/transcriptions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.audioTranscriptions',
    },
    {
      path: '/v1/audio/speech',
      method: 'POST' as const,
      descriptionKey: 'endpoints.audioSpeech',
    },
  ],
  utility: [
    {
      path: '/v1/models',
      method: 'GET' as const,
      descriptionKey: 'endpoints.models',
    },
    {
      path: '/v1/files',
      method: 'POST' as const,
      descriptionKey: 'endpoints.files',
    },
    {
      path: '/v1/batches',
      method: 'POST' as const,
      descriptionKey: 'endpoints.batches',
    },
  ],
} as const

export type EndpointCategory = keyof typeof endpointsConfig
