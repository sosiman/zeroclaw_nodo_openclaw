# Plan Completo: ZeroClaw Worker Node (3 Fases)

Este documento detalla la arquitectura y las fases de implementación para transformar ZeroClaw en un **Nodo Worker "Enterprise-Ready"** 100% compatible con el protocolo Gateway de OpenClaw, optimizado para hardware de 32 bits y bajos recursos (ej. 2GB RAM).

Se creará un nuevo subcomando `zeroclaw node --hub ws://[ip]:[puerto]` que levantará un módulo dedicado (`src/node/`), separado del Hub principal.

---

## FASE 1: MVP Interoperable (Conformance Test)
**Objetivo:** Demostrar el handshake WebSocket y la ejecución RPC básica contra un servidor OpenClaw real sin romper dependencias complejas.

### 1. Identidad y Capacidad (Pairing)
- **Generación ED25519:** Crear y persistir un par de claves.
- **Archivo Seguro:** Guardar metadatos (`node_id`, `protocol_version`, `created_at`) en `/var/lib/zeroclaw/node.json`. La clave privada debe tener **permisos estrictos `0600`**.
- **Negociación de Versión:** Enviar `minProtocol` y `maxProtocol` en el handshake (con lógica fallback/reject).

### 2. Handshake (`connect`)
- **Conexión WS:** Iniciar WebSocket hacia la IP del Hub.
- **Desafío (`connect.challenge`):** Firmar el `nonce` enviado por el Hub con la clave ED25519.
- **Declaración:** Enviar payload con `role: "node"`.
- **Anuncio de Capabilities:** Declarar lo que el nodo puede hacer (`can_run`, `can_invoke`, `sandbox_profile_default`, memory limits).

### 3. RPC Básico & Idempotencia
- **`nodes.run`:** Recibir un comando simple y devolver una **respuesta compatible real** (status alineado, stdout, stderr, y `exit_code`). Nada de "dummy responses".
- **`nodes.invoke`:** Manejar una llamada estructurada devolviendo un frame `res` correctamente empaquetado.
- **Idempotencia de Protocolo:** Toda respuesta DEBE incluir el `request_id` exacto del job original, garantizando la trazabilidad.

---

## FASE 2: Hardening & Sandbox
**Objetivo:** Añadir ejecución segura, control de flujo (streaming) y resiliencia de conexión.

### 1. Sandbox de Ejecución (3 Perfiles)
- **`safe`:** Allowlist estricta de comandos.
- **`ops`:** Comandos de administración limitados.
- **`full`:** Modo laboratorio (acceso total).
- **Límites Strictos:** 
  - Timeout obligatorio por cada comando.
  - Filtro severo de variables de entorno (ENV) y control del Current Working Directory (CWD).

### 2. Streaming Controlado & Límites Globales (¡Clave para 2GB RAM / 32-bit!)
- Enviar `stdout`/`stderr` en vivo al Hub, limitando el flujo por chunk y el total por job.
- **Límites Operativos Globales:**
  - `max_concurrent_jobs` (Para no saturar CPU).
  - `max_job_duration_sec` (Para prevenir procesos zombie).
  - `max_output_bytes_total` (Para proteger la RAM).

### 3. WebSocket Robusto
- **Reconexión:** Ciclo `exponential backoff` con **Jitter** (evita el "thundering herd" si caen 50 nodos a la vez).
- **Heartbeat:** Tarea de `ping`/`pong` para detectar caídas silenciosas de red.
- **Re-registro:** Auto-registro automático contra el Hub tras una reconexión exitosa.

---

## FASE 3: Enterprise Features (Persistence & Observability)
**Objetivo:** Persistencia real de trabajos, telemetría y control remoto.

### 1. Job Store Local (SQLite)
Base de datos ligera (`rusqlite`) para manejar colas e idempotencia real.
- **Esquema:** `request_id` (Unique Index), `status`, `started_at`, `ended_at`, `exit_code`, `retriable`, `attempt`.
- **Auditoría Extendida:** `device_id`, `command_hash`, `stdout_bytes`, `stderr_bytes`.

### 2. Ciclo de Vida de Identidad
- Añadir soporte para **rotación de claves** (key rotation) y comandos de **re-pairing** sin corromper el estado del nodo.

### 3. Observabilidad
- Telemetría básica exportable: `jobs_ok`, `jobs_fail`, `ws_latency_ms`, `ws_uptime`.

### 4. Modo Mantenimiento
- Comando de administración remota para "Drenar" (Drain) el nodo: deja de aceptar nuevos trabajos pero permite que los activos terminen limpiamente.

---

## Checklist de Verificación Final
1. [ ] Compila exitosamente en objetivos `i686` (32-bit x86) y `armv7`.
2. [ ] Ejecución `zeroclaw node --hub ws://192.168.0.50:18789` funciona.
3. [ ] El nodo aparece en el Hub de OpenClaw con las *capabilities* correctas.
4. [ ] Un comando `nodes.run` (`echo "Hello ZeroClaw"`) lanzado desde el Hub devuelve un stream real y el *exit code* correcto.
5. [ ] Un comando `nodes.invoke` responde con un frame `res` 100% compatible.
6. [ ] (Simulación): Al desconectar la red temporalmente, el *reconnect* con jitter automáticamene re-registra el nodo.
7. [ ] (Simulación): Lanzar comandos prohibidos devuelve bloqueo del Sandbox. Tareas lentas son aniquiladas por el Timeout.
8. [ ] Respuestas duplicadas con mismo `request_id` no re-ejecutan el job (idempotencia OK).
9. [ ] Se respetan límites globales (`max_concurrent_jobs`, `max_job_duration_sec`, `max_output_bytes_total`) sin ocasionar un OOM.
