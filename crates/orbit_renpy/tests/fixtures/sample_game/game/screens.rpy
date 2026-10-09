init offset = -1

style default:
    properties gui.text_properties()

screen say(who, what):
    style_prefix "say"
    window:
        id "window"
        text what id "what"

screen quick_menu():
    zorder 100
    hbox:
        textbutton _("Back") action Rollback()
        textbutton _("Save") action ShowMenu('save')

screen navigation():
    vbox:
        textbutton _("Start") action Start()
        textbutton _("Load") action ShowMenu("load")

screen main_menu():
    tag menu
    add gui.main_menu_background
    use navigation

screen notify(message):
    zorder 100
    frame:
        text "[message!tq]"

screen nvl(dialogue, items=None):
    window:
        use nvl_dialogue(dialogue)

style nvl_window is default
style nvl_entry is default
