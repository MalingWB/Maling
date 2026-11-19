A partir de esta ronda, adjunta el incremento del Forge Spec a tu respuesta. El incremento es el único contenido que el sistema analiza; todo lo demás es conversación.

Reglas de salida:

1. En la respuesta, el incremento de esta ronda va en un único bloque de código JSON; ningún otro JSON fuera de él;
2. Incluye solo los campos cambiados en esta ronda; nunca vuelques el Spec completo;
3. El incremento sigue esta estructura (los campos desconocidos se ignoran; los ausentes significan "sin cambios en esta ronda"):

```json
{
  "removedStageIds": ["ids de etapas a eliminar"],
  "name": "nombre del workflow tras converger (solo en rondas donde cambie)",
  "intent": "intención del proceso en una línea (solo en rondas donde cambie)",
  "workflowName": "nombre del workflow destino del incremento al dividir en varios",
  "stages": [
    {
      "id": "id de una etapa existente (omítelo para una nueva)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "objetivo de la etapa cubriendo los cuatro elementos",
      "roleDefinition": "carta completa del rol para roles custom (solo cuando role es custom; ver las reglas de forja abajo)",
      "profileId": "id del rol ya forjado asignado por el sistema — llévalo literal; nunca inventes, reescribas ni omitas",
      "artifact": "nombre del artefacto (cuando outputKind no es none)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "campo": "descripción del tipo" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["preguntas aún sin resolver que siguen en interrogatorio"]
}
```

4. Cada etapa en `stages` es el **estado nuevo completo** de la etapa (no un parche de campos): las etapas se alinean por `id` y se reemplazan por completo; una etapa sin `id` es nueva y se anexa;
5. **Reglas de forja de roles custom** (cuando role es custom):
   - escribe `role` como `{"custom": {"profileName": "Nombre del rol"}}` — una frase sustantiva concisa que nombre la profesión, no la etapa, y nunca duplicando los roles integrados (dev/test/review/plan/accept);
   - `roleDefinition` es la carta completa del rol, independiente de cualquier etapa: responsabilidades, método, límites, convenciones de salida — una persona profesional estable reutilizable entre workflows; no copies el goal de la etapa en ella;
   - cuando la etapa de la línea base ya lleva un `profileId`, ese rol ya está forjado: al ajustar nombre o carta en la conversación, cambia solo `profileName`/`roleDefinition` **llevando ese `profileId` literalmente** — el sistema actualiza el rol en su lugar por id y nunca lo duplica;
6. `openQuestions` reemplaza cada ronda la lista completa de pendientes; array vacío cuando todo converja;
7. Omite `artifact`/`schemaHint`/`successCondition` cuando `outputKind` sea `none`; nunca lleves contrato de artefacto cuando `onFailure` sea `manual-check` (revisión manual y validación de salida son mutuamente excluyentes);
8. Un workflow admite como máximo 10 etapas; al dividir, lleva las etapas del incremento bajo un nuevo `workflowName`.

Restricciones completas de los campos del Spec (JSON Schema):

{{ spec_schema }}

Línea base actual del Spec (los incrementos se fusionan en ella; vacía para una sesión nueva):

{{ current_spec }}
