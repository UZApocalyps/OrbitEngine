init offset = -2

init python:
    gui.init(1280, 720)

define gui.accent_color = '#cc6600'
define gui.text_size = 22
define gui.name_text_size = 30

style default:
    properties gui.text_properties()
    language gui.language

style input:
    properties gui.text_properties("input", accent=True)
    adjust_spacing False

style hyperlink_text:
    properties gui.text_properties("hyperlink", accent=True)
    hover_underline True
