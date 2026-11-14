# Open-hancom · Guía rápida

MCP local para ChatGPT Desktop · Windows de 64 bits

1. Extrae todo el ZIP público y cierra ChatGPT Desktop y Codex.
2. Ejecuta `clients\chatgpt\install.cmd`, sin permisos de administrador.
3. Abre ChatGPT, activa **open-hancom** y selecciona **Terra**, si está disponible.
4. Pega [PROMPT_ES.txt](PROMPT_ES.txt) en una conversación nueva. La conexión se confirma cuando `oh_guide` responde.

**Permisos:** autoriza las herramientas para consultar, crear o guardar según tu solicitud. No hace falta acceso a todo el disco ni desactivar la seguridad. Si la empresa bloquea el ejecutable, consulta a TI.

**Archivos:** copia los originales a `Documentos\OpenHancom\inbox`; los resultados quedan en `Documentos\OpenHancom\outbox`. Si Documentos está en OneDrive, se utiliza esa ubicación. Los adjuntos de la nube no se copian automáticamente al PC.

**Idioma:** los documentos HWPX y las hojas XLSX se generan en **COREANO**, con las plantillas existentes. Revisa el resultado con Hancom o un programa compatible.

**Si no aparece:** Settings → MCP servers → Add server → STDIO. Usa la ruta Binary indicada al instalar, con `mcp --library "<carpeta>\library" --workdir "<carpeta de trabajo>"`. Guarda y reinicia la aplicación. Si no existe ese menú, comprueba la compatibilidad de tu aplicación y cuenta.

El instalador guarda una copia de la configuración y actualiza solo `open-hancom`. ChatGPT Desktop y Codex comparten esa configuración en el mismo equipo. Cada PC ejecuta su propio servidor; los datos enviados a las herramientas y sus respuestas se procesan en ChatGPT.

El paquete público incluye un ejemplo de carta, sin formularios ni tarifas internas. Para usar otros formatos, registra tus propias plantillas o conecta un paquete autorizado.
