## The story.

label start:
    scene bg room with fade
    play music "audio/theme.ogg" fadein 1.0
    show eileen happy at left with dissolve
    e "Welcome to the {b}sample{/b} game!"
    show lucy at right, slide_in
    l "Hello there.{w} Nice {size=+10}big{/size} day."
    show eileen at midleft
    show eileen at Position(xpos=0.3)
    show eileen at Transform(zoom=0.8, xalign=0.6)
    show eileen at truecenter
    show eileen at offscreenright
    show lucy at bob
    show eileen:
        xalign 0.5 yalign 1.0
        linear 0.5 xalign 0.8 alpha 0.5
        pause 0.2
        ease 0.3 zoom 1.2 rotate 15
    show eileen:
        crop (0, 0, 100, 100)
    e "Let me show you around{nw}"
    extend " the place."
    e "Wait for it...{p}There!{fast}"
    voice "voice/e1.ogg"
    e "This line has a voice."
    window hide
    pause
    window show dissolve
    window auto
    $ renpy.pause(1.0, hard=True)
    $ renpy.notify("Saved!")
    $ renpy.music.play("audio/theme.ogg", channel="music", fadein=2.0)
    $ renpy.music.stop(fadeout=1.0)
    $ renpy.sound.play("audio/click.wav")
    $ achievement.grant("first_steps")
    $ persistent.seen_end = True
    $ persistent.visits = 1
    if persistent.met_lucy:
        l "We met before!"
    nvl clear
    nv "An NVL line."
    narrator_nvl "Narration in NVL mode."
    nvl clear
    g "I am your guide."
    show red_tint
    show fire
    show augustina happy
    show eileen hat
    show lamp
    with slow
    scene bg sea with flash
    scene bg night with shutter
    scene black_screen with pixellate
    with swipe
    with Dissolve(1.5)
    with Fade(0.5, 1.0, 0.5)
    with vpunch
    with moveinright
    jump town

label town:
    scene bg town
    "You arrive in town."
    call screen town_map
    jump town

label kitchen:
    scene bg kitchen
    "The kitchen."
    menu:
        "Go back":
            jump town
        "Look at the doors":
            call screen doors
    jump town

label garden:
    scene bg garden
    "The garden."
    $ achievement.grant("explorer")
    return

label ending:
    $ renpy.music.play("audio/theme.ogg")
    show screen town_buttons
    "The end."
    return

screen town_map():
    imagemap:
        ground "map ground.png"
        hover "map hover.png"
        hotspot (10, 20, 200, 100) action Jump("kitchen") alt "Kitchen"
        hotspot (300, 20, 200, 100) action Call("garden")
        if points > 2:
            hotspot (600, 20, 100, 100) action Jump("ending")
        hotspot (900, 600, 300, 100) action Return()

screen doors():
    imagebutton idle "door idle.png" hover "door hover.png" xpos 100 ypos 200 action Jump("garden")
    imagebutton:
        auto "door_%s.png"
        xpos 600
        ypos 200
        action [SetVariable("points", 3), Jump("ending")]

screen town_buttons():
    vbox:
        textbutton "Kitchen" action Jump("kitchen")
        textbutton "Garden" action Jump("garden")
