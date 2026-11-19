Tu incremento del Forge Spec más reciente no pasó la validación. Corrige el incremento y genéralo de nuevo; no cambies conclusiones que el usuario ya confirmó.

Errores estructurados (código → contexto):

{{ errors }}

Línea base del Spec que necesita reparo:

{{ spec }}

Requisitos de reparo:

1. Corrige solo los campos señalados por los códigos de error; deja todo lo demás como está;
2. Genera el incremento reparado con la misma estructura JSON de antes (un único bloque de código, sigue siendo un incremento — no el Spec completo);
3. Cuando el error provenga de algo que el usuario no aclaró (p. ej. `forge/missing-goal-element`), no fabriques contenido: pon la pregunta en `openQuestions` y pregúntala al usuario en la respuesta;
4. Si la validación sigue fallando varias rondas, lista todos los ítems sin resolver y devuelve la conversación al usuario en vez de emitir otro incremento.
