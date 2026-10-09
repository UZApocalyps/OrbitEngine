## Options of the sample game (a trimmed copy of the Ren'Py template).

define config.name = _("Sample Game")
define gui.show_name = True
define config.version = "1.0"
define build.name = "SampleGame"
define config.has_sound = True
define config.enter_transition = dissolve
define config.window = "auto"

init python:
    build.classify('**~', None)
    build.classify('**.bak', None)
    build.documentation('*.html')
