## Characters, transitions, images and transforms.

define e = Character("Eileen", color="#c8ffc8", what_color="#ffeedd", image="eileen")
define l = Character("Lucy", who_color="#ff8080", what_prefix='"', what_suffix='"', what_italic=True)
define nv = Character("Nora", kind=nvl, color="#8080ff")
define narrator_nvl = Character(None, kind=nvl)
define g = Character("Guide", ctc="ctc_blink", ctc_position="nestled")

define slow = Dissolve(2.0)
define flash = Fade(0.1, 0.0, 0.4, color="#fff")
define shutter = ImageDissolve("shutter.png", 1.0)
define swipe = MoveTransition(0.5)

image bg night = "bg/night.png"
image black_screen = Solid("#000")
image red_tint = Solid("#ff000080")
image eileen hat = Composite((600, 900), (0, 0), "images/eileen happy.png", (0, 0), "hat.png")
image lamp = ConditionSwitch("lamp_on", "lamp on.png", "True", "lamp off.png")
image bg sea = im.Scale("sea.jpg", 1280, 720)
image side eileen happy = "side eileen.png"

image fire:
    "fire 1.png"
    pause 0.1
    "fire 2.png"
    pause 0.1
    repeat

layeredimage augustina:
    always:
        "augustina_base"
    group outfit:
        attribute casual default
        attribute dress
    attribute happy

transform slide_in:
    xalign -0.5
    linear 1.0 xalign 0.0

transform bob:
    yoffset 0
    easein 0.5 yoffset -20
    easeout 0.5 yoffset 0
    repeat

transform midleft:
    xalign 0.25
    yalign 1.0

transform fade_in_out:
    alpha 0.0
    linear 0.5 alpha 1.0
    pause 1.0
    linear 0.5 alpha 0.0

transform shaky(amount):
    xoffset amount

default lamp_on = False
default persistent.seen_end = False
default points = 0

init python:
    achievement.register("first_steps")
    achievement.register("explorer", stat_max=3)
    config.keymap['screenshot'] = []
    def helper(x):
        return x * 2
