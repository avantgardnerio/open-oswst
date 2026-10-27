# Writing Apps

## Here's a the example app that should show the calls for most of the things you might want to do

You can do literally anything the Jumperless can in an app, so if there's a specific thing, lmk and I'll write an example. Until I make this into a proper operating system, what you're doing when you write an App is just writing a function in the main firmware. There's really no guard rails, and the API is just any function in the firmware.

If you don't want to recompile anything, you don't have to. A `.py` script saved on the board's file system is an app too, you just pick it from the `Files` row on the clickwheel and it runs. See [MicroPython](08-micropython.md) and the [File Manager](08-file-manager.md). From Python you can also run any of the built-in apps by name with `jumperless.run_app("Custom App")`.

## First get it PlatformIO set up to flash code

So fork the firmware here: https://github.com/Architeuthis-Flux/JumperlOS

I'm using PlatformIO in VSCode. Open the cloned `JumperlOS` folder itself, `platformio.ini` is at the root so the whole repo is the PlatformIO project.

The build environment is `jumperless_v5`. There's also `jumperless_v5_debug` (flashes over SWD), `jumperless_v5_erase` (wipes the file system while flashing) and `jumperless_og` for the original RP2040 Jumperless. `CodeDocs/CONTRIBUTING.md` has the `pio run` / `pio run -t upload` steps.

If your board doesn't come up on the port PlatformIO expects, comment out (or change) `upload_port = /dev/cu.usbmodemJLV5port1` and `monitor_port = /dev/cu.usbmodemJLV5port1` in `platformio.ini` so it'll just automatically find it. On macOS with a stock V5 that's the name it enumerates as, so you may not need to touch them.

You should probably try to just load the firmware just to make sure everything works.

## To write an App

Before you go writing your app, follow these steps to make it so it's listed in the App library and you can run it from the menus.

- Go to `src/remembering/menuTree.h` and add the name of your app under `"Apps"`. Each menu row is its own quoted string in the array, and an Apps entry looks like `"-Custom \31App"`. The leading `-` puts it one level under `Apps`, and the `\31` is the line break between the two halves. It needs to fit in 7x2 chars to show on the breadboard.

- Go to `src/Apps.h` and declare your function where you'll write your app, a plain `void yourApp(void);` next to the other prototypes. The body goes in `src/Apps.cpp`, or any `.cpp` under `src/`, every folder is on the include path.

- Go to `src/Apps.cpp` and add a row to `apps[NUM_APPS]` for your app: `{"Name", index, works, function}`. The name is up to 19 chars, `index` is informational, `works` has to be `1` or the app gets skipped with `App not found`, and `function` is the one `runApp` actually calls. The table is exactly full, so bump `NUM_APPS` in `src/Apps.h` by one for each row you add or it won't compile.

- The name in `menuTree.h` and the name in `apps[]` have to match. Picking the row on the clickwheel passes the row's own text to `runApp`, so `"-Custom \31App"` and `"Custom App"` line up once the `\31` is dropped and runs of spaces are collapsed. Case doesn't matter. If they don't match, the row just prints `App not found` and does nothing.

- You don't need to edit `runApp()` itself. It finds the app by name in `apps[]` (or by array position when you give it a number) and calls that row's function pointer.

- Make a function that's the entirety of your app, I just pushed a demo function called `customApp(void)` with some (non exhaustive) examples of things you can do from an app.

- Run your app with the clickwheel, `Apps > Custom App`.

From the terminal you run it from MicroPython with `jumperless.run_app("Custom App")` (the `>` prefix works, so `>run_app("Custom App")`). There's no digit shortcut.

If you want your own single-character shortcut, find an unused one and register it in `src/SingleCharCommands.cpp` with `registerCommand('<char>', "short description", "help text", callback, ...)`, where the callback calls `runApp(-1, (char*)"Your App Name")` and returns `CMD_DONT_SHOW_MENU`. Run it by name, not by number.

## To actually write the app

The code for `Custom App` is [`customApp()` in `src/Apps.cpp`](https://github.com/Architeuthis-Flux/JumperlOS/blob/main/src/Apps.cpp) (line 1358 in 5.7.11.0). It's mostly uncommented, but it shows the calls: `addBridgeToState` / `removeBridgeFromState` plus `refreshConnections`, `setTopRail`, `setDacByNumber`, `readAdcVoltage`, INA0 current and bus voltage, `probing.justReadProbe` and `checkProbeButtonState`, and `b.print` on the LEDs. Fair warning, it paints `Fuck    you!` on the breadboard at one point. There are tons more, but what's shown there are the higher-level helper functions that should roughly do what they say they're doing. If you want to skip the router entirely, `sendXYraw(chip, x, y, 1)` sets a single crosspoint on one of the crossbar chips directly (and 0 instead of 1 clears it), it's the same thing as `send_raw()` in MicroPython.

If your app changes connections, call `SlotManager::getInstance().enterTemporarySlot(8)` at the start and `leaveApp()` on every way out, so the user's slot comes back the way they left it.
