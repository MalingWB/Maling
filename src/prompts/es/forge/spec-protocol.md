A partir de esta ronda, adjunta el incremento del Forge Spec a tu respuesta. El incremento es el único contenido que el sistema analiza; todo lo demás es conversación.

Reglas de salida:

1. En la respuesta, el incremento de esta ronda va en un único bloque de código JSON; ningún otro JSON fuera de él;
2. Incluye solo los campos cambiados en esta ronda; nunca vuelques el Spec completo (cuando aún no haya línea de base, como en la primera ronda, el incremento es el borrador completo);
3. El incremento sigue esta estructura (los campos desconocidos se ignoran; los ausentes significan "sin cambios en esta ronda"):

```json
{
  "removedStageIds": ["ids de etapas a eliminar"],
  "name": "nombre del workflow (solo en rondas donde cambie)",
  "intent": "intención del proceso en una línea (solo en rondas donde cambie)",
  "workflowId": "id del workflow existente que se modifica (obligatorio si hay varios)",
  "workflowName": "nuevo nombre del workflow; sin workflowId, crea un workflow independiente",
  "parallelGroups": [{"id":"checks","branches":[{"id":"api","stages":["test-api"]},{"id":"ui","stages":["test-ui"]}],"maxParallel":3,"onFailure":"end"}],
  "stages": [
    {
      "id": "id de una etapa existente (omítelo para una nueva)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": {"input": "Requisitos", "output": "Informe", "boundary": "No desplegar", "acceptance": "Comprobaciones superadas"},
      "roleDefinition": "carta completa del rol para roles custom (solo para roles custom editables; ver las reglas de roles personalizados abajo)",
      "artifact": "nombre del artefacto (cuando outputKind no es none)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "campo": "descripción del tipo" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | end | new-round",
      "manualCheck": false
    }
  ],
  "openQuestions": [
    { "question": "una decisión que debe tomar el usuario", "recommendation": "la respuesta que ya pusiste en el borrador" }
  ]
}
```

4. Cada etapa en `stages` es el **estado nuevo completo** de la etapa (no un parche de campos): se alinea por `id` con una etapa existente de la "Línea base actual del Spec" y la reemplaza por completo; una etapa sin `id`, o con un `id` ausente de la línea de base, siempre es nueva y se anexa — para modificar una etapa existente debes llevar su `id`; las etapas nunca se alinean por posición; Si el objetivo del editor no cambia, conserva `goal.original` literalmente. Para crear o modificar un objetivo, usa las cuatro claves fijas del ejemplo, con valores no vacíos en el idioma del usuario. Si conservas una etapa del editor sin rol, mantén `role: "unassigned"`; no lo uses para etapas nuevas.
5. **`onFailure`** (a dónde va la etapa cuando falla) toma uno de estos valores:
   - `"retry"`: repetir la misma etapa (ejecutarla de nuevo);
   - `"manual-check"`: pausar y esperar al usuario;
   - `"end"`: terminar el workflow;
   - `"new-round"`: empezar una ronda nueva desde la primera etapa;
   - `{"new-round": "id de etapa"}`: empezar una ronda nueva desde la etapa indicada;
   - `{"goto": "id de etapa", "continueSession": true}`: volver a la etapa indicada; con `continueSession` a true, esa etapa conserva su sesión anterior para poder corregir su propio trabajo.

   El destino de un salto o de un reinicio debe ser un id de etapa existente en el mismo workflow;
6. **Reglas de roles personalizados** (cuando role es custom):
   - escribe `role` como `{"custom": {"profileName": "Nombre del rol"}}` — una frase sustantiva concisa que nombre la profesión, no la etapa, y nunca duplicando los roles integrados (dev/test/review/plan/accept);
   - `roleDefinition` es la carta completa del rol, independiente de cualquier etapa: responsabilidades, método, límites, convenciones de salida — una persona profesional estable reutilizable entre workflows; no copies el goal de la etapa en ella;
   - Para usar un rol que ya existe en este workflow, indica su `profileName`. Para renombrarlo, cambia todas las etapas que lo usan a la vez. No envíes `profileId`; el sistema resuelve la vinculación.
   - Las referencias a la biblioteca sin definición editable son de solo lectura: conserva el nombre y omite `roleDefinition`.
7. `openQuestions` reemplaza cada ronda la lista completa de pendientes: incluye solo las decisiones que realmente cambian el rumbo del flujo y que debe tomar el usuario, cada una con su recomendación; array vacío cuando todo converja;
8. Omite `artifact`/`schemaHint`/`successCondition` cuando `outputKind` sea `none`; nunca lleves contrato de artefacto cuando `onFailure` sea `manual-check` (revisión manual y validación de salida son mutuamente excluyentes);
9. Un workflow admite como máximo 10 etapas; al dividir, lleva las etapas del incremento bajo un nuevo `workflowName`. Usa `parallelGroups` para ramas independientes dentro del mismo flujo, sin dividirlo en varios flujos. Envía la lista completa de grupos solo si cambia su topología o su política; omitirla conserva los grupos y `[]` los elimina todos. El grupo ocupa la posición de su primer miembro en `stages`. Haz referencia a IDs de workers existentes (crea antes las etapas si es necesario). Cada worker pertenece a una sola rama, que ejecuta sus miembros en el orden de `stages`. Debe haber al menos dos ramas no vacías, sin anidamiento; `maxParallel` va de 1 a 5, normalmente 3. Si falla una etapa, solo puede terminar su rama o reintentar la misma etapa o una anterior de esa rama; no se permite revisión manual ni una ronda nueva. Cuando terminan todas las ramas, se avanza una sola vez si todas han tenido éxito; si no, se sigue el `onFailure` exterior del grupo sin reutilizar la sesión.
10. El nombre de un workflow debe ser único entre todos los workflows del usuario: guardar se rechaza si choca, así que elige un nombre que los distinga y pide al usuario que lo confirme;
11. La "Línea base actual del Spec" se adjunta al contexto de ejecución de cada mensaje y es la única fuente de verdad del Spec: léela al alinear un incremento. Las nuevas etapas no necesitan nombre tuyo — el sistema asigna su `id` tras fusionar y lo muestra en la siguiente línea de base.

Restricciones completas de los campos del Spec (JSON Schema):

{{ spec_schema }}
