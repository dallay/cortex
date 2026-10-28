//! Codificador incremental con acumulación acotada para las instantáneas finales.

use std::collections::{BTreeMap, HashSet};

use rook_core::{FinishReason, StreamChunk, TokenUsage};
use serde_json::{json, Value};

use crate::responses_adapter::{
    finish_status, message_item, text_part, ResponseContext, MAX_ARGUMENT_BYTES, MAX_OUTPUT_BYTES,
    MAX_TOOLS,
};

#[derive(Default)]
struct Tool {
    id: Option<String>,
    name: String,
    arguments: String,
    output_index: Option<usize>,
    emitted_bytes: usize,
}

pub(crate) struct ResponsesStream {
    context: ResponseContext,
    sequence: u64,
    output: Vec<Value>,
    text_index: Option<usize>,
    tools: BTreeMap<u32, Tool>,
    call_ids: HashSet<String>,
    output_bytes: usize,
    argument_bytes: usize,
    finish_reason: Option<FinishReason>,
    usage: Option<TokenUsage>,
}

fn event(sequence: &mut u64, kind: &str, mut data: Value) -> Value {
    data["type"] = json!(kind);
    data["sequence_number"] = json!(*sequence);
    *sequence += 1;
    data
}

impl ResponsesStream {
    pub fn new(context: ResponseContext) -> Self {
        Self {
            context,
            sequence: 0,
            output: vec![],
            text_index: None,
            tools: BTreeMap::new(),
            call_ids: HashSet::new(),
            output_bytes: 0,
            argument_bytes: 0,
            finish_reason: None,
            usage: None,
        }
    }

    pub fn start(&mut self) -> Vec<Value> {
        ["response.created", "response.in_progress"]
            .into_iter()
            .map(|kind| {
                event(
                    &mut self.sequence,
                    kind,
                    json!({"response":self.context.envelope("in_progress", vec![], None, None)}),
                )
            })
            .collect()
    }

    pub fn observe_usage(&mut self, usage: TokenUsage) {
        self.usage = Some(usage);
    }

    pub fn push(&mut self, chunk: StreamChunk) -> Result<Vec<Value>, &'static str> {
        // Revertir únicamente el frame rechazado, no copiar la salida acumulada.
        // Guardar longitudes evita copiar argumentos por cada delta recibido.
        let previous_indices: HashSet<_> = self.tools.keys().copied().collect();
        let previous_tools: Vec<_> = self
            .tools
            .iter()
            .filter(|(index, _)| {
                chunk.finish_reason.is_some()
                    || chunk.tool_calls.iter().any(|delta| delta.index == **index)
            })
            .map(|(index, tool)| {
                (
                    *index,
                    tool.id.clone(),
                    tool.name.len(),
                    tool.arguments.len(),
                    tool.output_index,
                    tool.emitted_bytes,
                )
            })
            .collect();
        let previous_ids = self.call_ids.clone();
        let previous_output_len = self.output.len();
        let previous_text_index = self.text_index;
        let previous_text_len = self.text_index.map(|index| {
            self.output[index]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .len()
        });
        let previous_sequence = self.sequence;
        let previous_output_bytes = self.output_bytes;
        let previous_argument_bytes = self.argument_bytes;
        let previous_finish = self.finish_reason;
        let result = self.push_frame(chunk);
        if result.is_err() {
            self.tools
                .retain(|index, _| previous_indices.contains(index));
            for (index, id, name_len, argument_len, output_index, emitted_bytes) in previous_tools {
                if let Some(tool) = self.tools.get_mut(&index) {
                    tool.id = id;
                    tool.name.truncate(name_len);
                    tool.arguments.truncate(argument_len);
                    tool.output_index = output_index;
                    tool.emitted_bytes = emitted_bytes;
                }
            }
            self.call_ids = previous_ids;
            self.output.truncate(previous_output_len);
            if let (Some(index), Some(len)) = (previous_text_index, previous_text_len) {
                if let Value::String(text) = &mut self.output[index]["content"][0]["text"] {
                    text.truncate(len);
                }
            }
            self.text_index = previous_text_index;
            self.sequence = previous_sequence;
            self.output_bytes = previous_output_bytes;
            self.argument_bytes = previous_argument_bytes;
            self.finish_reason = previous_finish;
            // El uso observado sí pertenece al intento, aunque se rechace el contenido.
        }
        result
    }

    fn push_frame(&mut self, chunk: StreamChunk) -> Result<Vec<Value>, &'static str> {
        if self.finish_reason.is_some()
            && (!chunk.delta.is_empty()
                || !chunk.tool_calls.is_empty()
                || chunk.finish_reason.is_some())
        {
            return Err("Contenido recibido después del estado final upstream");
        }
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
        }
        self.output_bytes = self.output_bytes.saturating_add(chunk.delta.len());
        if self.output_bytes > MAX_OUTPUT_BYTES {
            return Err("Se superó el límite de salida Responses");
        }
        let mut events = vec![];
        if !chunk.delta.is_empty() {
            let index = if let Some(index) = self.text_index {
                index
            } else {
                let index = self.output.len();
                let mut item = message_item(format!("msg_{}", self.context.id), "", "in_progress");
                item["content"] = json!([]);
                events.push(event(
                    &mut self.sequence,
                    "response.output_item.added",
                    json!({"output_index":index,"item":item}),
                ));
                item["content"] = json!([text_part("")]);
                self.output.push(item);
                self.text_index = Some(index);
                events.push(event(
                    &mut self.sequence,
                    "response.content_part.added",
                    json!({"output_index":index,
                    "item_id":self.output[index]["id"],"content_index":0,"part":text_part("")}),
                ));
                index
            };
            if let Value::String(text) = &mut self.output[index]["content"][0]["text"] {
                text.push_str(&chunk.delta);
            }
            events.push(event(&mut self.sequence, "response.output_text.delta", json!({"output_index":index,
                "item_id":self.output[index]["id"],"content_index":0,"delta":chunk.delta,"logprobs":[]})));
        }
        for delta in chunk.tool_calls {
            if !self.tools.contains_key(&delta.index) && self.tools.len() >= MAX_TOOLS {
                return Err("Se superó el límite de 64 llamadas de función");
            }
            let tool = self.tools.entry(delta.index).or_default();
            let coherent_id = delta
                .id
                .as_ref()
                .is_some_and(|id| tool.id.as_ref() == Some(id));
            if let Some(id) = delta.id {
                if !super::responses_adapter::valid_id(&id)
                    || tool.id.as_ref().is_some_and(|previous| previous != &id)
                {
                    return Err("El proveedor cambió la identidad de una función");
                }
                tool.id = Some(id);
            }
            if let Some(name) = delta.function.name.filter(|name| !name.is_empty()) {
                // Anthropic repite la identidad completa en su resumen terminal.
                let summary = chunk.finish_reason.is_some()
                    && coherent_id
                    && tool.name == name
                    && delta.function.arguments.is_none();
                if !summary {
                    if tool.output_index.is_some() {
                        return Err("El proveedor cambió el nombre de una función anunciada");
                    }
                    if tool.name.len().saturating_add(name.len()) > 64 {
                        return Err("Nombre de función demasiado largo");
                    }
                    tool.name.push_str(&name);
                }
            }
            if let Some(arguments) = delta.function.arguments {
                self.argument_bytes = self.argument_bytes.saturating_add(arguments.len());
                self.output_bytes = self.output_bytes.saturating_add(arguments.len());
                if self.argument_bytes > MAX_ARGUMENT_BYTES || self.output_bytes > MAX_OUTPUT_BYTES
                {
                    return Err("Se superó el límite de argumentos Responses");
                }
                tool.arguments.push_str(&arguments);
            }
            self.emit_tool(delta.index, chunk.finish_reason.is_some(), &mut events)?;
        }
        // Un finish separado también materializa funciones sin argumentos.
        if matches!(
            chunk.finish_reason,
            Some(FinishReason::Length | FinishReason::ContentFilter)
        ) {
            let pending: Vec<_> = self
                .tools
                .iter()
                .filter_map(|(index, tool)| tool.output_index.is_none().then_some(*index))
                .collect();
            for index in pending {
                self.emit_tool(index, true, &mut events)?;
            }
        }
        if let Some(reason) = chunk.finish_reason {
            self.finish_reason = Some(reason);
            self.validate_finish()?;
        }
        Ok(events)
    }

    fn emit_tool(
        &mut self,
        tool_index: u32,
        finalizing: bool,
        events: &mut Vec<Value>,
    ) -> Result<(), &'static str> {
        if let Some(tool) = self.tools.get_mut(&tool_index) {
            // Esperar al primer argumento permite ensamblar nombres fragmentados.
            if tool.output_index.is_none()
                && !tool.name.is_empty()
                && (!tool.arguments.is_empty() || finalizing)
            {
                if !super::responses_adapter::valid_name(&tool.name) {
                    return Err("Nombre de función inválido");
                }
                let call_id = tool
                    .id
                    .get_or_insert_with(|| format!("call_{}_{}", self.context.id, tool_index));
                if !self.call_ids.insert(call_id.clone()) {
                    return Err("Identidad de función duplicada");
                }
                let index = self.output.len();
                tool.output_index = Some(index);
                let item = json!({"type":"function_call","id":format!("fc_{}_{}",self.context.id,tool_index),
                    "call_id":call_id,"name":tool.name,"arguments":"","status":"in_progress"});
                events.push(event(
                    &mut self.sequence,
                    "response.output_item.added",
                    json!({"output_index":index,"item":item}),
                ));
                self.output.push(item);
            }
            if let Some(index) = tool.output_index {
                let arguments = &tool.arguments[tool.emitted_bytes..];
                if !arguments.is_empty() {
                    events.push(event(
                        &mut self.sequence,
                        "response.function_call_arguments.delta",
                        json!({"output_index":index,
                        "item_id":self.output[index]["id"],"delta":arguments}),
                    ));
                    tool.emitted_bytes = tool.arguments.len();
                }
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<Vec<Value>, &'static str> {
        self.validate_finish()?;
        let finish = self
            .finish_reason
            .ok_or("El stream terminó sin estado final upstream")?;
        let status = finish_status(Some(finish));
        for tool in self.tools.values() {
            if let Some(index) = tool.output_index {
                self.output[index]["arguments"] = json!(tool.arguments);
            }
        }
        let mut events = vec![];
        for (index, item) in self.output.iter_mut().enumerate() {
            item["status"] = json!(status);
            if item["type"] == "message" {
                events.push(event(&mut self.sequence, "response.output_text.done", json!({"output_index":index,
                    "item_id":item["id"],"content_index":0,"text":item["content"][0]["text"],"logprobs":[]})));
                events.push(event(
                    &mut self.sequence,
                    "response.content_part.done",
                    json!({"output_index":index,
                    "item_id":item["id"],"content_index":0,"part":item["content"][0]}),
                ));
            } else {
                events.push(event(
                    &mut self.sequence,
                    "response.function_call_arguments.done",
                    json!({"output_index":index,
                    "item_id":item["id"],"arguments":item["arguments"],"name":item["name"]}),
                ));
            }
            events.push(event(
                &mut self.sequence,
                "response.output_item.done",
                json!({"output_index":index,"item":item}),
            ));
        }
        events.push(event(&mut self.sequence, if status == "completed" { "response.completed" } else { "response.incomplete" },
            json!({"response":self.context.envelope(status, self.output.clone(), self.usage.as_ref(), Some(finish))})));
        Ok(events)
    }

    fn validate_finish(&self) -> Result<(), &'static str> {
        let finish = self
            .finish_reason
            .ok_or("El stream terminó sin estado final upstream")?;
        let status = finish_status(Some(finish));
        if finish == FinishReason::ToolCalls && self.tools.is_empty() {
            return Err("Estado tool_calls sin funciones");
        }
        if finish == FinishReason::Stop && !self.tools.is_empty() {
            return Err("Estado stop con funciones");
        }
        // Validar todo antes de publicar eventos done: un fallo no debe anunciar éxito.
        for tool in self.tools.values() {
            let Some(_) = tool.output_index else {
                return Err("Llamada de función incompleta");
            };
            if status == "completed"
                && !serde_json::from_str::<Value>(&tool.arguments).is_ok_and(|v| v.is_object())
            {
                return Err("Argumentos de función inválidos");
            }
        }
        Ok(())
    }

    pub fn fail(&mut self, message: &str) -> Vec<Value> {
        for tool in self.tools.values() {
            if let Some(index) = tool.output_index {
                self.output[index]["arguments"] = json!(tool.arguments);
            }
        }
        let error = json!({"code":"server_error", "message":message});
        let mut response =
            self.context
                .envelope("failed", self.output.clone(), self.usage.as_ref(), None);
        response["error"] = error;
        vec![
            event(
                &mut self.sequence,
                "error",
                json!({"code":"server_error","message":message,"param":null}),
            ),
            event(
                &mut self.sequence,
                "response.failed",
                json!({"response":response}),
            ),
        ]
    }
}

#[cfg(test)]
mod tests;
