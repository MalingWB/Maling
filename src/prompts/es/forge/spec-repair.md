El guardado falló; reparando {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Reparación del Forge Spec">
El incremento del Forge Spec que generaste en la última ronda no pasó la validación previa al guardado, tras fusionarse. Corrígelo y genera el incremento de nuevo; no cambies conclusiones que el usuario ya confirmó.

Errores estructurados (código → detalle, todos listados a la vez):

{{ errors }}

Requisitos de reparo:

1. Corrige solo las etapas y campos señalados por los errores; deja todo lo demás como está. Localiza la etapa con fallo en la "Línea base actual del Spec" y lleva su `id` al modificarla;
2. Genera el incremento reparado con la misma estructura JSON de antes (un único bloque de código, sigue siendo un incremento — no el Spec completo); corrige todos los problemas corregibles en esta misma ronda en vez de uno por ronda;
3. Corrige directamente los errores estructurales:
   - `forge/unknown-failure-target`: el destino del fallo apunta a una etapa que no existe. Cámbialo a `"retry"` y pon en `openQuestions`, con tu recomendación, la pregunta «¿a dónde debe ir esta etapa cuando falla?»;
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: Usa la misma definición de la línea base para todas las etapas con el mismo rol. Omite `profileId`; el sistema resuelve la vinculación.
   - `forge/readonly-role`: Las referencias a la biblioteca sin definición editable son de solo lectura: conserva el nombre y omite `roleDefinition`.
4. Nunca inventes la respuesta a algo que debe decidir el usuario: ponla en `openQuestions` con tu recomendación y dilo en la respuesta. Esto cubre:
   - `forge/missing-goal-element`, `forge/unresolved-questions`: falta un elemento en el objetivo de la etapa, o una pregunta abierta aún no se ha resuelto;
   - `forge/duplicate-workflow-name`: el nombre del workflow ya está en uso. Recomienda un nombre que los distinga y pide al usuario que lo confirme; no lo renombres por tu cuenta.
</hidden>
