# Biblioteca restrictiva de rutas

## Estado

`huginn-guards` expone una biblioteca de rutas. Todavía no se registra como plugin,
no está conectada a herramientas nativas y no protege run/chat/resume por defecto.
No incorpora API de shell, cargador de archivos ni integración con la CLI.

## Uso explícito

```rust
use huginn_guards::Guardrails;
use std::path::Path;

let policy = Guardrails::from_toml("block_paths=['private']")?;
policy.check_path(Path::new("/workspace"), Path::new("private/file"))?;
```

El host decide si invoca la biblioteca. Un resultado permitido no autoriza un
efecto ni sustituye `HostPolicy`, aprobación o validación del workspace.

## Configuración desde texto

`from_toml` acepta texto TOML de hasta 64 KiB y solo el campo `block_paths`.
Admite hasta 128 rutas de 1024 bytes, relativas, sin traversal ni controles.
Rechaza campos desconocidos: `block_commands`, `allow_paths`, `enabled` y `confirm`
no se aceptan ni se ignoran en esta capa. El proyecto solo añade restricciones.
La biblioteca no busca, abre ni carga un archivo de configuración del proyecto.

## Catálogo y resolución

Se bloquean `.env`, `.env.*`, `.ssh`, `.gnupg`, claves privadas `id_rsa`, `id_dsa`,
`id_ecdsa`, `id_ed25519`, `credentials`, `credentials.json`, `guardrails.toml` y
sufijos `.pem`, `.key`, `.p12` y `.pfx`. Las claves públicas como `id_rsa.pub`
no se bloquean por ese nombre. Los nombres y prefijos se comparan sin distinguir
mayúsculas ASCII; no se promete normalización Unicode ni coincidencia de substrings.
Los prefijos del proyecto respetan los límites entre componentes de ruta.

`check_path` comprueba la ruta solicitada y la resuelta. Normaliza componentes y
resuelve enlaces, incluido el ancestro existente de un destino todavía nuevo.
Los enlaces colgantes o una raíz no disponible producen un error.

## Límites de seguridad

No es un sandbox ni un mecanismo de autorización. No interpreta shell, scripts,
programas externos ni MCP. Tampoco detecta todos los alias mediante hardlinks.
La sustitución concurrente entre comprobación e I/O sigue sin resolverse: no hay
confinamiento atómico. La pertenencia al workspace y los efectos siguen siendo
responsabilidad del host y de sus adaptadores, que aún no usan esta biblioteca.
