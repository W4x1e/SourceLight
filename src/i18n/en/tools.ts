/** English strings for the tools area. See ../index.ts for the message format. */
export default {
  "tools.curve.point": "Curve point {n}",
  "tools.curve.pointValue": "input {input}, output {output}",
  "tools.curve.keysHint": "Arrow keys move the point, Shift for bigger steps. Delete removes it; Insert or + adds a point beside it.",
  "tools.mask.showMask": "Show what the mask covers",
  // NumberSlider
  "tools.slider.resetHint": "{label} — double-click to reset",
  // Added to the reset hint: Shift held while dragging moves the slider slowly.
  "tools.slider.fineHint": "Shift-drag for fine control",
  "tools.slider.valueLabel": "{label} value",

  // SpotPanel
  "tools.spot.title": "Spot removal",
  "tools.spot.intro": "Click a blemish to add a spot, then drag the hollow handle to choose where to copy from. {heal} blends with the surroundings; {clone} copies exactly.",
  "tools.spot.empty": "No spots yet — click the photo to add one.",
  "tools.spot.heal": "Heal",
  "tools.spot.clone": "Clone",
  // The list entry for a spot: its mode and its position in the list ("Heal 2").
  "tools.spot.itemName": "{mode} {number}",
  "tools.spot.delete": "Delete spot",
  "tools.spot.size": "Size",
  "tools.spot.feather": "Feather",
  // Undo-history entries
  "tools.spot.history.healMode": "Heal mode",
  "tools.spot.history.cloneMode": "Clone mode",
  "tools.spot.history.size": "Spot size",
  "tools.spot.history.feather": "Spot feather",

  // ErasePanel (magic erase, in Retouch)
  "tools.erase.title": "Erase",
  "tools.erase.intro": "Select an object, then erase it: the gap is filled from what surrounds it. Erase one thing at a time; large ones come out softer.",
  "tools.erase.click": "Click to select",
  "tools.erase.brush": "Brush",
  "tools.erase.clickPrompt": "Click the thing to erase.",
  "tools.erase.clickRefine": "Click again to take in more · Alt-click to leave part out.",
  "tools.erase.selecting": "Selecting…",
  "tools.erase.brushAdd": "Add",
  "tools.erase.brushRemove": "Take away",
  "tools.erase.brushSize": "Brush size",
  "tools.erase.run": "Erase",
  "tools.erase.clear": "Clear selection",
  "tools.erase.empty": "Nothing erased yet.",
  // The list entry for an erase: its position in the list ("Erase 2").
  "tools.erase.itemName": "Erase {number}",
  "tools.erase.delete": "Put back what this erased",

  // MaskPanel
  "tools.mask.title": "Masks",
  "tools.mask.selectSubjectTitle": "Auto-select the subject with AI",
  "tools.mask.selectingSubject": "Selecting subject…",
  "tools.mask.selectSubject": "Select subject",
  "tools.mask.pickTitle": "Click the photo to select an object; Alt-click to exclude",
  "tools.mask.donePicking": "Done picking",
  "tools.mask.clickToSelect": "Click to select",
  // What an AI mask run is doing, said in the masks panel and on the photo
  "tools.mask.stage.starting": "Getting ready…",
  "tools.mask.stage.waiting": "Waiting for tagging to free the graphics card…",
  "tools.mask.stage.loading": "Loading the AI model…",
  "tools.mask.stage.subject": "Finding the subject…",
  "tools.mask.stage.encoding": "Reading the photo…",
  "tools.mask.stage.refining": "Updating the selection…",
  "tools.mask.pickPrompt": "Click the thing you want to select.",
  "tools.mask.pickRefine": "Click again to refine · Alt-click to exclude a region.",
  "tools.mask.newMask": "New mask",
  "tools.mask.shape.radial": "Radial",
  "tools.mask.shape.linear": "Linear",
  "tools.mask.shape.brush": "Brush",
  // The tonal-range mask type: selects by brightness
  "tools.mask.shape.luminance": "Range",
  "tools.mask.shape.colorRange": "Colour",
  "tools.mask.shape.ai": "AI",
  "tools.mask.newShapeTitle": "New {shape} mask",
  "tools.mask.addShapeButton": "+ {shape}",
  "tools.mask.empty": "No masks yet. Add one, drag it into place on the photo, then make your adjustments below.",
  "tools.mask.renameHint": "Double-click to rename",
  "tools.mask.rename": "Rename mask",
  "tools.mask.delete": "Delete mask",
  // The default name of a mask with no name: its first shape and its position ("Radial 2")
  "tools.mask.defaultName": "{shape} {number}",
  "tools.mask.fallbackName": "Mask",
  "tools.mask.subject": "Subject",
  // A mask name with the number of its components after it: "Radial 1 (2)"
  "tools.mask.nameWithCount": "{name} ({count})",
  "tools.mask.components": "Components",
  "tools.mask.base": "Base",
  "tools.mask.mode.add": "Add",
  "tools.mask.mode.subtract": "Subtract",
  "tools.mask.mode.intersect": "Intersect",
  // A component's shape name, marked as inverted ("Radial · inv")
  "tools.mask.invertedName": "{shape} · inv",
  "tools.mask.invertTitle": "Invert this component",
  "tools.mask.deleteComponent": "Delete component",
  "tools.mask.addComponentTitle": "Add {shape} component (combine with add/subtract/intersect)",
  "tools.mask.feather": "Feather",
  "tools.mask.brushHelp": "Drag on the photo to paint. Turn on Erase to remove.",
  "tools.mask.brushSize": "Brush size",
  "tools.mask.hardness": "Hardness",
  "tools.mask.erasing": "Erasing",
  "tools.mask.erase": "Erase",
  "tools.mask.clearBrush": "Clear brush",
  "tools.mask.luminanceHelp":
    "Selects pixels by brightness. Drag Low/High to set the tonal band; Feather softens each edge.",
  "tools.mask.low": "Low",
  "tools.mask.high": "High",
  "tools.mask.rangeHistory": "Range {label}",
  "tools.mask.colorHelp":
    "Selects pixels by colour. Hue picks the target; Range widens the band of hues; Feather softens the edge. Neutral greys are never selected.",
  "tools.mask.hue": "Hue",
  "tools.mask.range": "Range",
  "tools.mask.colourHistory": "Colour {label}",
  "tools.mask.aiHelp": "An AI selection. Refine it by adding Brush or Range components.",
  "tools.mask.localHistory": "Local {label}",
  // The heading over the selected mask's own sliders.
  "tools.mask.adjustHeading": "Adjust this mask",
  "tools.mask.adj.exposure": "Exposure",
  "tools.mask.adj.contrast": "Contrast",
  "tools.mask.adj.highlights": "Highlights",
  "tools.mask.adj.shadows": "Shadows",
  "tools.mask.adj.temperature": "Temperature",
  "tools.mask.adj.tint": "Tint",
  "tools.mask.adj.saturation": "Saturation",
  "tools.mask.adj.whites": "Whites",
  "tools.mask.adj.blacks": "Blacks",
  "tools.mask.adj.clarity": "Clarity",
  "tools.mask.adj.texture": "Texture",
  "tools.mask.overlayHideTitle": "Hide the mask overlay to see the edit underneath",
  "tools.mask.overlayShowTitle": "Show the mask overlay",

  // CropPanel
  "tools.crop.title": "Crop and rotate",
  "tools.crop.doneTitle": "Leave the crop tool (R)",
  "tools.crop.intro": "Drag a corner or an edge to crop, and inside the frame to move it.",
  "tools.crop.aspectRatio": "Aspect ratio",
  "tools.crop.free": "Free",
  // The photo's own shape before cropping
  "tools.crop.original": "Original",
  "tools.crop.rotateFlip": "Rotate and flip",
  "tools.crop.rotateLeftTitle": "Left 90°",
  "tools.crop.rotateLeft": "Left",
  "tools.crop.rotateRightTitle": "Right 90°",
  "tools.crop.rotateRight": "Right",
  "tools.crop.flipTitle": "Flip horizontally",
  "tools.crop.flip": "Flip",
  "tools.crop.straighten": "Straighten",
  "tools.crop.reset": "Reset crop",
  "tools.crop.resetTitle": "Undo the crop, rotation and flip",
  // Undo-history entries
  "tools.crop.history.rotateLeft": "Rotate left",
  "tools.crop.history.rotateRight": "Rotate right",
  "tools.crop.history.flip": "Flip",

  // AiDenoise
  "tools.denoise.title": "AI noise reduction",
  "tools.denoise.amount": "Amount",
  "tools.denoise.timeLeft": "About {duration} left. You can keep editing.",
  "tools.denoise.took": "About {duration} on this computer.",
  "tools.denoise.notRun": "Set to {percent}% but not applied to the preview yet. Exports still include it.",
  "tools.denoise.runNow": "Run now",
  "tools.denoise.reduce": "Reduce noise",
  "tools.denoise.blurb": "Removes high-ISO noise while keeping detail.",
  "tools.denoise.defaultEstimate":
    "Takes a few seconds on a graphics card, a minute or two on a laptop's built-in graphics.",

  // denoiseTime
  "tools.denoise.seconds": { one: "{count} s", other: "{count} s" },
  "tools.denoise.minutes": { one: "{count} min", other: "{count} min" },

  // CurveEditor
  "tools.curve.hint": "Click to add a point, drag to shape, double-click to remove.",

  // GradingWheels
  "tools.grading.threeWay": "3-way",
  "tools.grading.focus": "Focus",
  "tools.grading.range.shadows": "shadows",
  "tools.grading.range.midtones": "midtones",
  "tools.grading.range.highlights": "highlights",
  "tools.grading.refine": "Refine {range}",
  "tools.grading.luminance": "Luminance",
  "tools.grading.saturation": "Saturation",
  "tools.grading.hue": "Hue",
  // Undo-history entries; {range} is shadows, midtones or highlights
  "tools.grading.history.grade": "{range} grade",
  "tools.grading.history.luminance": "{range} luminance",
  "tools.grading.history.saturation": "{range} saturation",
  "tools.grading.history.hue": "{range} hue",
  "tools.grading.history.reset": "Reset {range} colour",
  // A colour wheel's name for assistive tech; {range} is shadows, midtones or highlights.
  "tools.grading.wheelLabel": "{range} colour",
  "tools.grading.wheelValue": "hue {hue}°, saturation {saturation}",
  "tools.grading.wheelHint": "Drag to choose a colour · arrow keys to nudge · double-click to reset",

  // ClickSelectOverlay
  "tools.pick.include": "Include",
  "tools.pick.exclude": "Exclude",
} as const;
