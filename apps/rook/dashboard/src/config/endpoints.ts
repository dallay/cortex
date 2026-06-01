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
      path: '/chat/completions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.chatCompletions',
    },
    {
      path: '/responses',
      method: 'POST' as const,
      descriptionKey: 'endpoints.responses',
    },
    {
      path: '/completions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.completions',
    },
    {
      path: '/messages',
      method: 'POST' as const,
      descriptionKey: 'endpoints.messages',
    },
  ],
  media: [
    {
      path: '/embeddings',
      method: 'POST' as const,
      descriptionKey: 'endpoints.embeddings',
    },
    {
      path: '/images/generations',
      method: 'POST' as const,
      descriptionKey: 'endpoints.imagesGenerations',
    },
    {
      path: '/audio/transcriptions',
      method: 'POST' as const,
      descriptionKey: 'endpoints.audioTranscriptions',
    },
    {
      path: '/audio/speech',
      method: 'POST' as const,
      descriptionKey: 'endpoints.audioSpeech',
    },
  ],
  utility: [
    {
      path: '/models',
      method: 'GET' as const,
      descriptionKey: 'endpoints.models',
    },
    {
      path: '/files',
      method: 'POST' as const,
      descriptionKey: 'endpoints.files',
    },
    {
      path: '/batches',
      method: 'POST' as const,
      descriptionKey: 'endpoints.batches',
    },
  ],
} as const

export type EndpointCategory = keyof typeof endpointsConfig
