# dev/ — Docker-based E2E Testing

Corré los tests de integración en contenedores Docker aislados, sin ensuciar tu sistema.

## Distros disponibles

| Distro   | Base image       | Props                      |
|----------|------------------|----------------------------|
| `ubuntu` | Debian Bookworm  | Compatibilidad máxima       |
| `alpine` | Alpine 3.20      | Imagen más pequeña (~50MB) |

## Quick start

```bash
# 1. Build las imágenes (solo una vez o cuando cambie el código)
dev/e2e-test.sh build

# 2. Levantar los containers
dev/e2e-test.sh up

# 3. Correr tests en todos los distros
dev/e2e-test.sh test

# 4. O probar solo ubuntu
dev/e2e-test.sh test ubuntu

# 5. Ver el /health de alpine
dev/e2e-test.sh health alpine

# 6. Entrar a un container
dev/e2e-test.sh shell ubuntu

# 7. Bajar todo
dev/e2e-test.sh down

# 8. Limpiar imágenes y containers
dev/e2e-test.sh clean
```

## Tests que cubre

| Task | Descripción                                    |
|-------|------------------------------------------------|
| 6.4   | `/health` devuelve JSON válido con `status` + `providers` |
| 6.5   | Registry vacío → `no_providers_configured` sin errores |
| 6.6   | CRUD create → refresh → routing e2e (manual) |

## Para test 6.6 (CRUD e2e manual)

Con los containers arriba, probá a mano:

```bash
# Ubuntu
curl -X POST http://localhost:8081/api/providers \
  -H "Content-Type: application/json" \
  -d '{"providerKind":"openai","name":"test","apiKey":"sk-test","priority":1}'

# Listar
curl http://localhost:8081/api/providers

# Obtener uno
curl http://localhost:8081/api/providers/<id>

# Eliminar
curl -X DELETE http://localhost:8081/api/providers/<id>
```

## Notas

- **No usa volumes** — la base de datos es `:memory:`. Cada reinicio del container parte de cero.
- `provider_crud.enabled = false` en el config por defecto — los endpoints de CRUD no están montados. Para probar CRUD, cambiá `test-configs/rook-minimal.toml` a `enabled = true` y agregá las variables de encryption.
- Los logs del container: `docker compose -f dev/docker-compose.yml logs -f rook-ubuntu`
- Si necesitás reconstruir después de cambios en código: `dev/e2e-test.sh build`
