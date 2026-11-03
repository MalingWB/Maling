## Novedades y ajustes

### 1. Intervención remota y notificaciones mediante Feishu

Ahora puedes aprobar permisos, responder a preguntas del Agent, realizar comprobaciones manuales y recibir notificaciones de resultados mediante Feishu, incluso cuando estás lejos del ordenador.

Para empezar:

1. Abre «Configuración → General → Intervención remota y notificaciones por IM», selecciona Feishu, confirma los permisos y escanea el código QR para conectarte.
2. Envía un mensaje privado al bot desde la cuenta que autorizó la conexión mediante el código QR para completar la vinculación.
3. Cuando recibas una tarjeta, podrás aprobar o denegar permisos, responder con una o varias opciones, texto o una respuesta personalizada, o realizar una comprobación manual.

Después de resolver una solicitud en la aplicación de escritorio, WeCom o Feishu, el resultado se sincroniza con los demás canales conectados que tengan activada la notificación correspondiente. Las tarjetas de Feishu conservan las respuestas realmente enviadas y desactivan los controles ya utilizados.

Por ahora, solo se admiten conversaciones privadas individuales con la cuenta vinculada, y cada bot puede conectarse a un único dispositivo. No se admiten chats de grupo ni el inicio, la detención o la repetición de tareas por IM. No se garantiza el envío y la recepción en tiempo real cuando el cliente está desconectado, el ordenador está suspendido o no hay conexión de red. La versión internacional, Lark, aún no se ha validado.

> [!WARNING]
> **Antes de actualizar, revisa las limitaciones de las tarjetas antiguas y de la vuelta a versiones anteriores**
> Esta actualización elimina los registros de entrega de las tarjetas interactivas de IM antiguas y sus vínculos de mensajes asociados. Algunas tarjetas recibidas antes de actualizar pueden dejar de funcionar; resuelve las solicitudes pendientes desde la aplicación de escritorio. Los clientes antiguos no pueden leer directamente los datos de IM actualizados. Si necesitas conservar la posibilidad de volver a una versión anterior, haz una copia de seguridad de los datos de la aplicación antes de actualizar.

### 2. Mejoras en la visualización de sub-Agents nativos de Claude y tareas en segundo plano

Cuando Claude inicia un sub-Agent, puedes consultar su estado en la fila Agent de la conversación y abrir detalles de solo lectura en el espacio de trabajo de la derecha. Las subtareas ejecutadas de forma síncrona o en segundo plano muestran sus propios mensajes, solicitudes de permiso y estados. El final del turno principal ya no hace que las subtareas que siguen ejecutándose se marquen incorrectamente como interrumpidas.

La actividad en segundo plano tiene un contador de tiempo independiente, lo que reduce la inclusión de largos periodos de inactividad en el tiempo de ejecución. También puedes copiar y citar con normalidad las respuestas una vez terminada la actividad en segundo plano.

> [!NOTE]
> Se necesita Claude ACP `0.90.0` o posterior; las versiones antiguas muestran un aviso para actualizar. La continuación de sub-Agents existentes después de recargar la sesión sigue dependiendo de las capacidades del adaptador. El total de Token no incluye el consumo de las reactivaciones en segundo plano; el panel muestra una advertencia sobre esta limitación.

### 3. Mejoras en la ejecución de requisitos remotos y la transferencia de resultados

La página «Requisitos» de la edición interna (WB) mejora la gestión de tareas de desarrollo, tareas de prueba y archivos adjuntos remotos:

1. Consulta y ejecuta los requisitos por tareas de desarrollo o de prueba. Si la tarea de desarrollo correspondiente aún no ha terminado, la tarea de prueba recomendará esperar, pero seguirá permitiendo ejecutarla manualmente.
2. Al pulsar «Ejecutar», el contenido del requisito, las instrucciones de entrega de tareas anteriores y otros datos de contexto se incorporan a la conversación. La tarea solo se asume oficialmente cuando pulsas «Enviar».
3. Antes de enviar, puedes consultar una lista de solo lectura de los archivos adjuntos remotos. Se descargan al enviar y se utilizan como entradas de la tarea. El fallo al descargar un archivo no bloquea la tarea, y se informa al Agent del contenido que falta.
4. El modo Direct espera una salida de finalización explícita antes de comunicar que la tarea ha terminado, evitando cerrar el requisito tras la primera respuesta. La etapa final del flujo de trabajo resume las entregas de todo el elemento de trabajo para que las utilicen las tareas posteriores.

### 4. Sincronización bidireccional de Skills remotos con varios archivos

Ahora puedes sincronizar Skills completos entre el entorno local y Multica, incluidos `SKILL.md` y archivos de apoyo, como scripts y documentación. En la página de gestión global de Skills, pulsa «Sincronizar SKILL remotos», conéctate a Multica y selecciona un espacio de trabajo y un Skill remoto para descargarlo y consultar el resultado de la sincronización. Para importar un Skill local al entorno remoto, inicia la operación desde Multica.

Se han corregido los problemas que conservaban selecciones antiguas al recargar la lista remota y dejaban la recuperación incompleta tras errores de sincronización.

> [!CAUTION]
> **La descarga con sobrescritura sustituye todo el directorio del Skill**
> Al elegir sobrescribir un Skill local, prevalece el contenido del directorio remoto. También se eliminarán los archivos locales que no existan en el remoto. Guarda antes los cambios locales que quieras conservar.

### 5. Atajo para abrir la búsqueda de conversaciones

Pulsa `Ctrl+F`, o `Cmd+F` en macOS, para abrir la búsqueda de conversaciones y enfocar el campo de búsqueda. Al pulsar de nuevo, se selecciona el término actual. Pulsa `Esc` para cerrar y devolver el foco al control anterior.

El editor de código mantiene la prioridad de su propia búsqueda dentro del archivo. La búsqueda de conversaciones no se abre encima de otros diálogos, menús o selectores que ya estén abiertos.

### 6. Confirmación del destino antes de hacer push en Git

Antes de hacer push, el control de código fuente muestra el destino real con el formato «rama local → remoto / rama de destino», para que puedas comprobar adónde se enviará el código. Al publicar una rama por primera vez, puedes elegir el remoto y definir su upstream. Las ramas que ya tienen upstream hacen push al destino que realmente siguen.

Las ramas sin upstream se etiquetan como «Nueva rama», para evitar confundir las ramas aún no publicadas con las que ya están sincronizadas.

## Mejoras de experiencia

### 1. Lectura del historial y recuperación de sesiones más estables

Se ha corregido el problema por el que el desplazamiento automático te devolvía al final mientras leías mensajes anteriores. También puedes cargar más historial cuando la página actual no llena la pantalla. Al volver a una sesión, las preguntas ya respondidas no regresan al estado pendiente por datos antiguos en caché.

Al continuar una sesión, se usan el modelo y los permisos realmente guardados en ella, lo que reduce los fallos provocados por configuraciones antiguas del flujo de trabajo.

### 2. Los archivos adjuntos con el mismo nombre ya no sobrescriben el historial

Los archivos adjuntos con nombres iguales y contenidos distintos se guardan por separado. Las nuevas subidas no sobrescriben archivos anteriores ni sus vistas previas. Se ha mejorado el guardado de nombres largos, con un aviso claro cuando la ruta es demasiado profunda para almacenar el archivo de forma segura.

### 3. Mejoras en Markdown y el espacio de trabajo de archivos

1. Se han corregido las vistas previas de imágenes locales y la apertura de enlaces HTML locales en las conversaciones, conservando los parámetros de consulta y las anclas de la página.
2. La vista previa en tiempo real de Markdown aprovecha el ancho disponible del panel, reduciendo los márgenes laterales innecesarios en pantallas anchas.
3. El tamaño de fuente predeterminado del editor en los temas integrados pasa a `13px`. Se conservan los tamaños personalizados existentes.
4. Si se produce un error de renderizado en una pestaña de la derecha, puedes reintentar en esa pestaña sin afectar a toda la conversación. También se ha mejorado la estabilidad cuando hay varios árboles de archivos al mismo tiempo.

### 4. Corrección del recuento duplicado de Token

Se ha corregido el recuento duplicado del consumo de Token al continuar una conversación tras un periodo de inactividad. Las duplicaciones identificables también se corrigen al leer registros existentes.

### 5. Otras mejoras

1. Se ha corregido el fallo al cargar las estadísticas de cambios al consultar commits con muchos archivos o commits de merge en Windows.
2. Se ha mejorado la detección automática del idioma chino: los marcadores explícitos de chino simplificado o tradicional tienen prioridad sobre la región. Se conservan las elecciones manuales de idioma existentes.
3. Se han completado las traducciones relacionadas con las sesiones mantenidas activas y se ha corregido el cierre tardío de algunas ayudas que aparecen al pasar el ratón.
