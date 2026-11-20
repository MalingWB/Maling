## Funciones nuevas y actualizadas

> [!IMPORTANT]
> Esta versión migra la configuración local. La versión 0.21.x no puede leer la configuración migrada. Si necesitas conservar la opción de volver a una versión anterior, haz una copia de seguridad de los datos de la aplicación antes de actualizar.

### 1. Reintenta el nodo actual o vuelve a ejecutar desde un nodo fallido en el gráfico del flujo de trabajo

Abre el gráfico de ejecución del flujo de trabajo en la conversación y haz clic con el botón derecho sobre un nodo que cumpla los requisitos:

1. **Reintentar nodo actual**: si el nodo actual se ha pausado por un problema y el flujo aún no ha avanzado, vuelve a ejecutarlo dentro de la misma ejecución (Run). Se conservan la conversación y los artefactos anteriores.
2. **Volver a ejecutar desde este nodo**: selecciona un registro de ejecución que haya finalizado con un fallo, incluidos los registros anteriores que admiten esta acción. Un nuevo Run comienza en ese nodo con la configuración más reciente del flujo guardada en la tarea. El Run original queda disponible como historial de solo lectura.

> [!WARNING]
> Volver a ejecutar desde un nodo fallido detiene lo que siga en curso en el Run original. No deshace cambios del espacio de trabajo, commits de Git ni acciones externas ya realizadas. Ambas opciones se limitan a nodos normales del flujo de trabajo y no se aplican al modo AUTO ni a nodos internos de AI-DYNAMIC.

### 2. Continúa o recupera flujos de trabajo desde Feishu y WeCom

Tras vincular un chat privado en Feishu o WeCom, la aplicación envía una tarjeta de acción cuando el flujo de trabajo se pausa en un estado que admite esta función. Esto incluye los flujos que se detienen manualmente en la aplicación de escritorio:

1. **Continuar flujo de trabajo**: reanuda el nodo actual si aún no ha terminado.
2. **Recuperar flujo de trabajo**: si el nodo actual terminó correctamente pero el flujo todavía no avanzó, usa el resultado existente para continuar sin volver a ejecutar ese nodo.

Las notificaciones están activadas de forma predeterminada. En **Configuración → General → Intervención remota y notificaciones de IM** puedes desactivar **Solicitud para continuar el flujo** para cada canal. Si la tarjeta indica que la aplicación sigue ocupada justo después de detener el flujo, espera un momento y vuelve a intentarlo en la misma tarjeta.

> [!NOTE]
> La aplicación de escritorio debe estar en línea. Esta actualización no permite iniciar, detener ni volver a ejecutar flujos de trabajo mediante IM. La aplicación no envía tarjetas en bloque para flujos que ya se habían pausado antes de actualizar.

### 3. Tres nuevos Agents y una configuración inicial más flexible

- **Gestión de Agent** incorpora DeepSeek Harness, ZCode y Qoder. ZCode usa un adaptador de la comunidad. La opción integrada de Qoder no está disponible en Windows ARM64. El nombre visible por defecto de la opción integrada CodeBuddy pasa a ser WorkBuddy; se conservan las configuraciones existentes.
- Al añadir o editar un Agent, consulta **Requisitos de conexión** para comprobar los comandos locales y los métodos de autenticación necesarios.
- La configuración inicial sigue comprobando Node.js, npm y npx, pero si falla alguna comprobación todavía se pueden detectar los Agents locales. Los resultados distinguen entre los que no están instalados, los que no tienen el entorno de ejecución necesario y los que no superaron el diagnóstico. Cuando la detección termine o falle, elige **Configurar más tarde** para entrar en la pantalla principal y completar la configuración después, aunque no haya ningún Agent disponible.

### 4. Temas Vista y Meadow, con fondos de pantalla separados para los modos claro y oscuro

Elige uno de los dos temas nuevos en **Configuración → Apariencia**:

- **Vista**: una pintura panorámica en acuarela para el modo claro, y cumbres nevadas en negro y dorado bajo un cielo estrellado para el modo oscuro. Los tonos verde azulado y dorado albaricoque proceden de los dos fondos de pantalla.
- **Meadow**: el modo claro usa un verde tierno más suave que el blanco puro; el modo oscuro evoca una noche tranquila de primavera con verde pino. El código y las vistas de Diff usan fondos más claros para facilitar la lectura.

Gold Band, Vista y Meadow incluyen seis fondos de pantalla en total. En cada modo puedes elegir **Fondo del tema**, **Personalizado** o **Desactivar**. Los fondos personalizados admiten **Rellenar** y **Ajustar** y pueden eliminarse de **Usados recientemente**.

### 5. Salta entre turnos y ajusta la altura del cuadro de mensaje

- Usa la nueva navegación por turnos a la izquierda de la conversación. Pasa el puntero por un marcador para ver una vista previa del mensaje o haz clic para saltar al turno, incluidos los turnos anteriores que aún no aparecen en pantalla.
- En las pantallas de inicio y de conversación, arrastra el borde superior del cuadro de mensaje para cambiar su altura. La altura se conserva al cambiar de conversación. Arrastra el borde hacia abajo hasta una posición baja para volver al ajuste automático.

### 6. Desinstala roles y flujos de trabajo locales desde Multica

Una vez establecida la conexión, abre la pestaña **Runtime** en la gestión de roles de Multica. Allí puedes ver los roles y flujos de trabajo locales del entorno de ejecución actual y desinstalar elementos que no estén integrados. La desinstalación solo elimina la copia local; no borra el contenido del catálogo compartido de Multica. Cuando envías varias operaciones sobre recursos seguidas, la aplicación de escritorio procesa antes las solicitudes en cola.

## Mejoras de experiencia

### 1. Barra lateral y conversaciones más fáciles de usar

- Desde **Recientes**, puedes desplegar el historial de ejecuciones, cambiar el nombre de una conversación o fijarla. Usa los botones al final de las listas de la barra lateral para cargar más elementos. Al actualizar el historial se conserva la selección, y el indicador de actividad desaparece cuando termina la tarea.
- El cuerpo de las tablas de Markdown usa el color de texto normal del tema, lo que mejora la lectura tanto en temas claros como oscuros.

### 2. Inicio y conexiones con Agents más estables

- Si la configuración de inicio de un Agent no es válida, ya no impide abrir la aplicación. Se ha corregido un problema por el que WorkBuddy podía cambiar de método de inicio en turnos posteriores e interrumpir una conexión local que funcionaba.
- Se han corregido algunos fallos de inicio de WebView. Los entornos no compatibles ahora muestran indicaciones de actualización para la plataforma correspondiente. Los clientes MCP más recientes detectan con mayor fiabilidad las herramientas integradas de memoria compartida.

### 3. Base de conocimientos y menú de descargas en el sitio web oficial

El sitio web oficial y el Demo ahora usan la marca MALING. Puedes consultar la base de conocimientos en el sitio o usar el menú de descargas para elegir el instalador de tu plataforma.
