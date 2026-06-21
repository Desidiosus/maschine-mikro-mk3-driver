# Bitwig Studio Integration

DAW integration for the Maschine Mikro MK3 driver, built on top of the driver's virtual
MIDI ports. This is optional — the driver and configuration GUI work with any MIDI software
on their own. See the [main README](../README.md) for driver and GUI setup.

Run the shell commands below from the repository root.

A controller script is included for full Bitwig integration. Copy it to your Bitwig
controller scripts folder:

```shell
mkdir -p ~/Bitwig\ Studio/Controller\ Scripts/MaschineMikroMK3
cp bitwig/MaschineMikroMK3.control.js ~/Bitwig\ Studio/Controller\ Scripts/MaschineMikroMK3/
```

To rebuild the Bitwig script from source, use Node.js 24 LTS. The `bitwig/` directory
includes `.nvmrc` and `engine-strict=true`, so run `nvm use` inside `bitwig/` before
`npm install`, `npm update`, or `npm run build`.

```shell
cd bitwig
nvm use
npm install
npm run build
```

## Connecting to Bitwig (PipeWire/ALSA)

Bitwig uses ALSA **Raw MIDI** devices directly (not the ALSA sequencer), so you need to
route through Virtual Raw MIDI. The driver can auto-connect to `snd-virmidi` on startup
when the raw-MIDI bridge is enabled. This is the one setting not exposed in the GUI; put
it in a config file and start the driver with `-c`:

```toml
[bridge]
midi_bridge_virmidi = true
autoconnect_virmidi = true
virmidi_client_name = ""
virmidi_port = 0
```

```shell
cargo run -p driver --release -- -c your_config.toml
```

Then in Bitwig:

1. Go to **Settings → Controllers → Add Controller**
2. Select **Native Instruments → Maschine Mikro MK3 (Linux)**
3. Set Input to **Virtual Raw MIDI/1**
4. Set Output to **Virtual Raw MIDI/2**
5. (Optional) Click the controller name to customise pad LED feedback settings

### Optional: rename "Virtual Raw MIDI" to "Maschine Mikro MK3"

The `snd-virmidi` kernel module supports renaming via the `id=` parameter:

```shell
sudo modprobe -r snd_virmidi snd_seq_virmidi
sudo modprobe snd-virmidi midi_devs=2 id="Maschine Mikro MK3"
```

## Screen Display

The driver exposes the OLED screen as a SysEx text protocol on the virtual MIDI input. The
Bitwig script uses it to show contextual information in real time:

- **Mode name** when switching modes
- **Track name** in Play and Mixer modes
- **Note name** when changing the step sequencer note
- **Feature status** when toggling Note Repeat or Fixed Velocity

## Mode System

The controller supports four operational modes, each providing different functionality for
the pads and other controls. The current mode is displayed on the OLED screen.

| Mode | Button | Pad Function | Encoder Function |
|------|--------|--------------|------------------|
| **Play** | Keyboard | Play notes (normal) | Navigate tracks |
| **Step** | Step | Toggle sequencer steps | Change step note |
| **Clip** | Scene | Launch clips/scenes | Navigate scenes |
| **Mixer** | Pattern | Track controls (select/mute/solo/arm) | Navigate tracks |

**Switching Modes:**

- Press **Pad Mode** to cycle through modes
- Press **Keyboard**, **Step**, **Scene**, or **Pattern** to jump directly to that mode
- Press **Shift + Pad Mode** to return to Play mode
- Hold **Shift** + mode button for the original view toggle function

## Note Repeat

Press **Note Repeat** to enable auto-retriggering of held pad notes. While enabled, holding
a pad continuously retriggers that note at the selected interval.

| Action | Function |
|--------|----------|
| Note Repeat | Toggle note repeat on/off |
| Shift + Note Repeat | Cycle repeat rate (1/16 → 1/8 → 1/4) |

## Fixed Velocity

Press **Fixed Vel** to force all pad hits to a fixed velocity (100 by default). Useful for
consistent drum programming or uniform note levels.

| Action | Function |
|--------|----------|
| Fixed Vel | Toggle fixed velocity on/off |
| Shift + Fixed Vel | Show current fixed velocity value |

## Step Sequencer Mode

In Step mode, the 16 pads represent 16 steps in a drum sequencer pattern:

- **Press a pad** to toggle that step on/off
- **Rotate encoder** to change the note being sequenced (C1 to G9)
- **Press Erase** to clear all steps
- **Yellow pads** = steps with notes
- **White pad** = current playhead position
- **Off pads** = empty steps

The step sequencer edits the cursor clip in your current track.

## Mixer Mode

In Mixer mode, pads control the first 4 tracks in a grid layout:

| Row | Function | Colors |
|-----|----------|--------|
| Top (1-4) | Select track | Blue |
| Row 2 (5-8) | Toggle mute | Orange |
| Row 3 (9-12) | Toggle solo | Yellow |
| Bottom (13-16) | Toggle arm | Red |

## Clip Launcher Mode

In Clip mode, pads trigger clips and scenes:

| Row | Function |
|-----|----------|
| Top row (1-4) | Launch scenes 1-4 |
| Other rows | Launch track clips |

## Button Functions in Bitwig

| Button | Function | Shift + Button |
|--------|----------|----------------|
| Play | Play/Pause | Return to arrangement |
| Stop | Stop | Reset automation |
| Rec | Toggle record | Toggle overdub |
| Restart | Jump to start | Toggle loop |
| Tap | Tap tempo | Toggle metronome |
| Left | Rewind | Previous track |
| Right | Fast forward | Next track |
| Browse | Open/close browser | Insert device after |
| Encoder | Mode-specific (see above) | Navigate tempo |
| Encoder Press | Select in editor | Select in mixer |
| Solo | Toggle solo | - |
| Mute | Toggle mute | - |
| Sampling | Toggle arm | - |
| Volume | Undo | Redo |
| Follow | Zoom to selection | Zoom to fit |
| Duplicate | Duplicate | Duplicate object |
| Erase | Delete (or clear steps in Step mode) | Cut |
| Plugin | Next device | Previous device |
| Slider | Track volume | - |
| Note Repeat | Toggle note repeat | Cycle repeat rate |
| Fixed Vel | Toggle fixed velocity | Show velocity value |
| Pad Mode | Cycle modes | Return to Play mode |
| Keyboard | Play mode | Toggle note editor |
| Step | Step mode | Toggle automation editor |
| Scene | Clip mode | Toggle mixer |
| Pattern | Mixer mode | Return to arrangement |

## Pad Playback Feedback (Bitwig 6+)

The controller script includes visual feedback on pads during clip/sequence playback. This
feature uses the `playingNotes()` API introduced in Bitwig Studio 6 beta. The script works
on older versions but without playback feedback.

**Example use case:** When a drum loop is playing in a clip, the pads light up in sync with
the beat, showing exactly which drums are being triggered — handy for visualising the rhythm
and jamming along live.

### Customizable Settings

Go to **Settings → Controllers → Maschine Mikro MK3 (Linux)** to customise:

| Setting | Options | Description |
|---------|---------|-------------|
| **Playback Feedback** | Enabled / Disabled | Show visual feedback for notes playing from clips |
| **Manual Hit Feedback** | Enabled / Disabled | Show visual feedback when you press pads manually |
| **Playback Color Mode** | Track Color / Fixed Color | Use track color or a fixed color for playback |
| **Fixed Playback Color** | Red, Orange, Yellow, Green, Cyan, Blue, Purple, Magenta, White | Color to use when Fixed Color mode is selected |
| **Manual Hit Color** | Red, Orange, Yellow, Green, Cyan, Blue, Purple, Magenta, White | Color for manually pressed pads (default: Blue) |

**Track Color Mode:** Each track has its own color in Bitwig, making it easy to identify
which track/drums are active.

**Fixed Color Mode:** All playback uses the same color regardless of track — useful if you
prefer consistency.
