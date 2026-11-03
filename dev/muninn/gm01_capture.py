"""Productor GM-01 importable; el llamador controla cliente y aislamiento."""

from copy import deepcopy
import json

from gm01_differential import CaptureError, compare


class RustBridgeAdapter:
    """Lee clientes genéricos; escribe solo con OwnedFixture viva del runner.

    El runner construye y verifica el binario antes de emitir esta capacidad.
    No es una frontera contra código Python hostil dentro del mismo proceso.
    """

    def __init__(self, client):
        self.client = client

    async def add_message(self, session, role, content, metadata):
        from gm01_runner import OwnedFixture
        if type(self.client) is not OwnedFixture:
            raise CaptureError('Captura Rust bloqueada: requiere fixture propia para verificar el binario actual y permitir capturas sin embeddings')
        try:
            self.client.assert_live()
        except RuntimeError as error:
            raise CaptureError('Captura Rust bloqueada: fixture no activa o modificada') from error
        return self.client.call('add_message', session_id=session, role=role,
                                content=content, metadata=deepcopy(metadata),
                                generate_embedding=False)

    async def get_conversation(self, session):
        conversation = self.client.call('get_conversation', session_id=session)
        fields = ('id', 'conversation_id', 'role', 'content', 'metadata', 'timestamp')
        messages = []
        for message in conversation['messages']:
            for field in fields:
                if field not in message:
                    raise CaptureError(f'Respuesta Rust sin {field}')
            messages.append({field: deepcopy(message[field]) for field in fields})
        return {'id': conversation['id'], 'session_id': conversation['session_id'],
                'messages': messages}


class ReferenceAdapter:
    """Adapta MemoryClient de la referencia fijada, sin gestionar su ciclo de vida."""

    def __init__(self, client):
        self.client = client

    async def add_message(self, session, role, content, metadata):
        return await self.client.short_term.add_message(
            session, role, content, metadata=metadata,
            extract_entities=False, extract_relations=False,
            generate_embedding=False, extraction_mode='skip')

    async def get_conversation(self, session):
        conversation = await self.client.short_term.get_conversation(session)
        return {
            'id': str(conversation.id), 'session_id': conversation.session_id,
            'messages': [{
                'id': str(message.id), 'conversation_id': str(message.conversation_id),
                'role': message.role.value, 'content': message.content,
                'metadata': deepcopy(message.metadata), 'timestamp': message.created_at.isoformat(),
            } for message in conversation.messages],
        }


async def capture_gm01(adapter, sessions):
    """Añade el mismo escenario a dos sesiones explícitas y devuelve sus lecturas."""
    if (not isinstance(sessions, (tuple, list)) or len(sessions) != 2
            or any(not isinstance(s, str) or not s.strip() for s in sessions)
            or sessions[0] == sessions[1]):
        raise ValueError('GM-01 requiere dos sesiones distintas no vacías')
    metadata = {'id': 'conservar', 'timestamp': 'conservar', 'nested': {'tags': ['ñ', 1]}}
    scenario = (
        (('system', 'Instrucción GM-01'), ('user', 'Hola, Muñinn 🧠\nsegunda línea'),
         ('assistant', ''), ('user', 'x' * 10001), ('user', 'duplicado'), ('user', 'duplicado')),
        (('assistant', 'otra sesión'),),
    )
    conversations = []
    for session, messages in zip(sessions, scenario, strict=True):
        for role, content in messages:
            await adapter.add_message(session, role, content, deepcopy(metadata))
        conversations.append(await adapter.get_conversation(session))
    capture = {'conversations': conversations}
    verdict = compare(capture, capture)
    if verdict['comparison'] != 'pass':
        raise CaptureError('; '.join(verdict['errors']))
    for session, messages, conversation in zip(sessions, scenario, conversations, strict=True):
        expected = [{'role': role, 'content': content, 'metadata': metadata}
                    for role, content in messages]
        actual = [{field: message[field] for field in ('role', 'content', 'metadata')}
                  for message in conversation['messages']]
        # JSON distingue bool/int también dentro de metadata; no normaliza listas.
        if (conversation['session_id'] != session
                or json.dumps(actual, ensure_ascii=False, sort_keys=True, allow_nan=False)
                != json.dumps(expected, ensure_ascii=False, sort_keys=True, allow_nan=False)):
            raise CaptureError('La lectura no coincide con la sesión y el escenario escritos')
    return capture
