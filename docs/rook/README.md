# Rook — AI Proxy and Router

Rook es el componente principal de Cortex: un proxy y router de IA de alto rendimiento escrito en Rust.

## Guía rápida

```bash
# Run Rook
cargo run -p rook

# O con just
just run

# Health check
curl http://127.0.0.1:3773/health
```

## Estructura de documentación

```
rook/
├── README.md              ← Este archivo
├── architecture.md        ← Arquitectura interna
├── api.md                 ← Referencia de API HTTP
├── configuration.md       ← Guía de configuración
├── providers.md           ← Configuración de proveedores
├── deployment.md          ← Guía de despliegue
└── adr/                   ← Architecture Decision Records
    └── README.md
```

## Documentación

### Primeros pasos
1. [Configuración](configuration.md) — Cómo configurar Rook
2. [API Reference](api.md) — Endpoints disponibles
3. [Providers](providers.md) — Configurar modelos de IA

### Desarrollo
- [Arquitectura](architecture.md) — Diseño interno y capas
- [Despliegue](deployment.md) — Producción y contenedores

### Decisiones de arquitectura
- [ADR Index](adr/README.md) — Decisiones técnicas de Rook

## Conceptos clave

### Proxy y Router
Rook actúa como un proxy inteligente entre tu aplicación y múltiples proveedores de IA (OpenAI, Anthropic, Ollama, etc.). Permite:
- Routing dinámico por modelo, proveedor o prioridad
- Circuit breaker para resiliencia
- Rate limiting por tier

### API Compatible
Rook expone una API OpenAI-compatible (`/v1/chat/completions`) que transparenta la complejidad de múltiples proveedores.

### Dashboard
El dashboard Vue SPA está embebido en el binario de Rook bajo `/dashboard/` y permite gestión visual de providers y API keys.

## Recursos adicionales

- [README principal](../../README.md) — Overview del proyecto Cortex
- [Dashboard docs](../../apps/rook/dashboard/README.md) — Desarrollo del dashboard
- [Desarrollo local](../development.md) — Setup y testing
