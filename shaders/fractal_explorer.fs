/*{
    "DESCRIPTION": "Free flight through a six-slot hybrid 3D fractal with Mandelbulb3D-style formulas.",
    "CREDIT": "Varda",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "throttle", "LABEL": "Throttle", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "flight_mode", "LABEL": "Flight", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Walk", "Dive"]},
        {"NAME": "look_heading", "LABEL": "Heading", "TYPE": "float", "DEFAULT": 0.0, "MIN": -180.0, "MAX": 180.0},
        {"NAME": "look_pitch", "LABEL": "Pitch", "TYPE": "float", "DEFAULT": 0.0, "MIN": -90.0, "MAX": 90.0},
        {"NAME": "look_roll", "LABEL": "Roll", "TYPE": "float", "DEFAULT": 0.0, "MIN": -180.0, "MAX": 180.0, "GROUP": "Camera"},
        {"NAME": "yaw_rate", "LABEL": "Yaw Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "GROUP": "Camera"},
        {"NAME": "pitch_rate", "LABEL": "Pitch Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "GROUP": "Camera"},
        {"NAME": "roll_rate", "LABEL": "Roll Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "GROUP": "Camera"},
        {"NAME": "strafe_x", "LABEL": "Strafe X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "GROUP": "Camera"},
        {"NAME": "strafe_y", "LABEL": "Strafe Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "GROUP": "Camera"},
        {"NAME": "fov", "LABEL": "Field of View", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 2.5, "GROUP": "Camera"},
        {"NAME": "reset_camera", "LABEL": "Reset Camera", "TYPE": "bool", "DEFAULT": false, "GROUP": "Camera"},
        {"NAME": "find_inside", "LABEL": "Find Inside", "TYPE": "bool", "DEFAULT": false, "GROUP": "Camera"},
        {"NAME": "autopilot", "LABEL": "Autopilot", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Motion"},
        {"NAME": "save_location", "LABEL": "Save Location", "TYPE": "bool", "DEFAULT": false, "GROUP": "Motion"},
        {"NAME": "location", "LABEL": "Location", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 31.0, "GROUP": "Motion"},
        {"NAME": "recall_time", "LABEL": "Recall Seconds", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Motion"},
        {"NAME": "tour", "LABEL": "Tour Locations", "TYPE": "bool", "DEFAULT": false, "GROUP": "Motion"},
        {"NAME": "tour_seconds", "LABEL": "Seconds per Stop", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 60.0, "GROUP": "Motion"},
        {"NAME": "hybrid_mode", "SPECIALIZE": true, "LABEL": "Hybrid", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Alternate", "Combine"], "GROUP": "Form"},
        {"NAME": "repeat_from", "SPECIALIZE": true, "LABEL": "Repeat From Slot", "TYPE": "long", "DEFAULT": 1, "VALUES": [1, 2, 3, 4, 5, 6], "LABELS": ["1", "2", "3", "4", "5", "6"], "GROUP": "Form"},
        {"NAME": "max_iterations", "LABEL": "Iterations", "TYPE": "float", "DEFAULT": 16.0, "MIN": 1.0, "MAX": 64.0, "GROUP": "Form"},
        {"NAME": "bailout", "LABEL": "Bailout", "TYPE": "float", "DEFAULT": 100.0, "MIN": 2.0, "MAX": 1000.0, "GROUP": "Form"},
        {"NAME": "julia_mode", "LABEL": "Julia", "TYPE": "bool", "DEFAULT": false, "GROUP": "Form"},
        {"NAME": "julia_x", "LABEL": "Julia X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "GROUP": "Form"},
        {"NAME": "julia_y", "LABEL": "Julia Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "GROUP": "Form"},
        {"NAME": "julia_z", "LABEL": "Julia Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "GROUP": "Form"},
        {"NAME": "combine_split", "SPECIALIZE": true, "LABEL": "Part 2 From Slot", "TYPE": "long", "DEFAULT": 4, "VALUES": [2, 3, 4, 5, 6], "LABELS": ["2", "3", "4", "5", "6"], "GROUP": "Form"},
        {"NAME": "combine_op", "LABEL": "Combine", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Union", "Intersect", "Subtract", "Chamfer", "Fillet"], "GROUP": "Form"},
        {"NAME": "combine_width", "LABEL": "Combine Width", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 64.0, "GROUP": "Form"},
        {"NAME": "slot1_formula", "SPECIALIZE": true, "LABEL": "Slot 1 Formula", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 1"},
        {"NAME": "slot1_mode", "SPECIALIZE": true, "LABEL": "Slot 1 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 1"},
        {"NAME": "slot1_count", "SPECIALIZE": true, "LABEL": "Slot 1 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 1"},
        {"NAME": "slot1_a", "LABEL": "Slot 1 A", "TYPE": "float", "DEFAULT": 2.2, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 1"},
        {"NAME": "slot1_b", "LABEL": "Slot 1 B", "TYPE": "float", "DEFAULT": 0.5, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 1"},
        {"NAME": "slot1_c", "LABEL": "Slot 1 C", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 1"},
        {"NAME": "slot1_d", "LABEL": "Slot 1 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 1"},
        {"NAME": "slot1_rot_x", "LABEL": "Slot 1 Rotate X", "TYPE": "float", "DEFAULT": 0.25, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 1"},
        {"NAME": "slot1_rot_y", "LABEL": "Slot 1 Rotate Y", "TYPE": "float", "DEFAULT": 0.15, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 1"},
        {"NAME": "slot1_rot_z", "LABEL": "Slot 1 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 1"},
        {"NAME": "slot2_formula", "SPECIALIZE": true, "LABEL": "Slot 2 Formula", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 2"},
        {"NAME": "slot2_mode", "SPECIALIZE": true, "LABEL": "Slot 2 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 2"},
        {"NAME": "slot2_count", "SPECIALIZE": true, "LABEL": "Slot 2 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 2"},
        {"NAME": "slot2_a", "LABEL": "Slot 2 A", "TYPE": "float", "DEFAULT": 0.15, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 2"},
        {"NAME": "slot2_b", "LABEL": "Slot 2 B", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 2"},
        {"NAME": "slot2_c", "LABEL": "Slot 2 C", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 2"},
        {"NAME": "slot2_d", "LABEL": "Slot 2 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 2"},
        {"NAME": "slot2_rot_x", "LABEL": "Slot 2 Rotate X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 2"},
        {"NAME": "slot2_rot_y", "LABEL": "Slot 2 Rotate Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 2"},
        {"NAME": "slot2_rot_z", "LABEL": "Slot 2 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 2"},
        {"NAME": "slot3_formula", "SPECIALIZE": true, "LABEL": "Slot 3 Formula", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 3"},
        {"NAME": "slot3_mode", "SPECIALIZE": true, "LABEL": "Slot 3 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 3"},
        {"NAME": "slot3_count", "SPECIALIZE": true, "LABEL": "Slot 3 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 3"},
        {"NAME": "slot3_a", "LABEL": "Slot 3 A", "TYPE": "float", "DEFAULT": 2.0, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 3"},
        {"NAME": "slot3_b", "LABEL": "Slot 3 B", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 3"},
        {"NAME": "slot3_c", "LABEL": "Slot 3 C", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 3"},
        {"NAME": "slot3_d", "LABEL": "Slot 3 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 3"},
        {"NAME": "slot3_rot_x", "LABEL": "Slot 3 Rotate X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 3"},
        {"NAME": "slot3_rot_y", "LABEL": "Slot 3 Rotate Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 3"},
        {"NAME": "slot3_rot_z", "LABEL": "Slot 3 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 3"},
        {"NAME": "slot4_formula", "SPECIALIZE": true, "LABEL": "Slot 4 Formula", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 4"},
        {"NAME": "slot4_mode", "SPECIALIZE": true, "LABEL": "Slot 4 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 4"},
        {"NAME": "slot4_count", "SPECIALIZE": true, "LABEL": "Slot 4 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 4"},
        {"NAME": "slot4_a", "LABEL": "Slot 4 A", "TYPE": "float", "DEFAULT": 2.0, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 4"},
        {"NAME": "slot4_b", "LABEL": "Slot 4 B", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 4"},
        {"NAME": "slot4_c", "LABEL": "Slot 4 C", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 4"},
        {"NAME": "slot4_d", "LABEL": "Slot 4 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 4"},
        {"NAME": "slot4_rot_x", "LABEL": "Slot 4 Rotate X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 4"},
        {"NAME": "slot4_rot_y", "LABEL": "Slot 4 Rotate Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 4"},
        {"NAME": "slot4_rot_z", "LABEL": "Slot 4 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 4"},
        {"NAME": "slot5_formula", "SPECIALIZE": true, "LABEL": "Slot 5 Formula", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 5"},
        {"NAME": "slot5_mode", "SPECIALIZE": true, "LABEL": "Slot 5 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 5"},
        {"NAME": "slot5_count", "SPECIALIZE": true, "LABEL": "Slot 5 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 5"},
        {"NAME": "slot5_a", "LABEL": "Slot 5 A", "TYPE": "float", "DEFAULT": 2.0, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 5"},
        {"NAME": "slot5_b", "LABEL": "Slot 5 B", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 5"},
        {"NAME": "slot5_c", "LABEL": "Slot 5 C", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 5"},
        {"NAME": "slot5_d", "LABEL": "Slot 5 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 5"},
        {"NAME": "slot5_rot_x", "LABEL": "Slot 5 Rotate X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 5"},
        {"NAME": "slot5_rot_y", "LABEL": "Slot 5 Rotate Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 5"},
        {"NAME": "slot5_rot_z", "LABEL": "Slot 5 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 5"},
        {"NAME": "slot6_formula", "SPECIALIZE": true, "LABEL": "Slot 6 Formula", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22], "LABELS": ["Empty", "Box", "Menger", "Sierpinski", "KIFS", "Pseudo-Kleinian", "Kaliset", "Mandelbulb", "Transform", "Helispiral", "Gnarl", "Bulbox P-2", "Sphere Inversion", "Polyfold Sym", "Sine", "Reciprocal", "Repeat", "Koch Cube", "JCube", "Lin Combine", "Rotate 4D", "ABoxMod2", "msltoe Sym4"], "GROUP": "Slot 6"},
        {"NAME": "slot6_mode", "SPECIALIZE": true, "LABEL": "Slot 6 Variant", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Variant 1", "Variant 2", "Variant 3", "Variant 4", "Variant 5"], "GROUP": "Slot 6"},
        {"NAME": "slot6_count", "SPECIALIZE": true, "LABEL": "Slot 6 Iterations", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Slot 6"},
        {"NAME": "slot6_a", "LABEL": "Slot 6 A", "TYPE": "float", "DEFAULT": 2.0, "MIN": -4.0, "MAX": 16.0, "GROUP": "Slot 6"},
        {"NAME": "slot6_b", "LABEL": "Slot 6 B", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 6"},
        {"NAME": "slot6_c", "LABEL": "Slot 6 C", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 6"},
        {"NAME": "slot6_d", "LABEL": "Slot 6 D", "TYPE": "float", "DEFAULT": 1.0, "MIN": -4.0, "MAX": 4.0, "GROUP": "Slot 6"},
        {"NAME": "slot6_rot_x", "LABEL": "Slot 6 Rotate X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 6"},
        {"NAME": "slot6_rot_y", "LABEL": "Slot 6 Rotate Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 6"},
        {"NAME": "slot6_rot_z", "LABEL": "Slot 6 Rotate Z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Slot 6"},
        {"NAME": "detail", "LABEL": "Detail (px)", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.25, "MAX": 4.0, "GROUP": "Detail"},
        {"NAME": "lod", "LABEL": "Geometry Band (px)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "GROUP": "Detail"},
        {"NAME": "max_steps", "LABEL": "Ray Steps", "TYPE": "float", "DEFAULT": 200.0, "MIN": 32.0, "MAX": 512.0, "GROUP": "Detail"},
        {"NAME": "step_mult", "LABEL": "Step Size", "TYPE": "float", "DEFAULT": 0.9, "MIN": 0.3, "MAX": 1.0, "GROUP": "Detail"},
        {"NAME": "temporal", "LABEL": "Temporal Smoothing", "TYPE": "bool", "DEFAULT": true, "GROUP": "Detail"},
        {"NAME": "tile_prepass", "LABEL": "Tile Prepass", "TYPE": "bool", "DEFAULT": true, "GROUP": "Detail"},
        {"NAME": "render_scale", "LABEL": "Max Render Scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.35, "MAX": 1.0, "GROUP": "Detail"},
        {"NAME": "target_fps", "LABEL": "Target FPS", "TYPE": "float", "DEFAULT": 50.0, "MIN": 0.0, "MAX": 144.0, "GROUP": "Detail"},
        {"NAME": "still_samples", "LABEL": "Still Samples", "TYPE": "float", "DEFAULT": 32.0, "MIN": 4.0, "MAX": 64.0, "GROUP": "Detail"},
        {"NAME": "sharpness", "LABEL": "Sharpness", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.0, "MAX": 1.0, "GROUP": "Detail"},
        {"NAME": "debug_view", "LABEL": "Diagnostic", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8], "LABELS": ["Off", "Steps", "Normals", "Depth", "Exit Cause", "Iterations", "Parity Grid", "Shadow and Occlusion", "Shade Reuse"], "GROUP": "Detail"},
        {"NAME": "look", "LABEL": "Look", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Custom", "Stone Hall", "Desert Sunbeams", "Moonlit", "Teal & Gold"], "GROUP": "Lighting"},
        {"NAME": "sun_elev", "LABEL": "Sun Elevation", "TYPE": "float", "DEFAULT": 0.6, "MIN": -1.5, "MAX": 1.5, "GROUP": "Lighting"},
        {"NAME": "sun_azim", "LABEL": "Sun Azimuth", "TYPE": "float", "DEFAULT": 2.4, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Lighting"},
        {"NAME": "sun_color", "LABEL": "Sun Color", "TYPE": "color", "DEFAULT": [1.0, 0.92, 0.8, 1.0], "GROUP": "Lighting"},
        {"NAME": "sun_intensity", "LABEL": "Sun Intensity", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 16.0, "GROUP": "Lighting"},
        {"NAME": "shadow_softness", "LABEL": "Shadow Softness", "TYPE": "float", "DEFAULT": 12.0, "MIN": 1.0, "MAX": 64.0, "GROUP": "Lighting"},
        {"NAME": "shadow_length", "LABEL": "Shadow Length", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.5, "MAX": 16.0, "GROUP": "Lighting"},
        {"NAME": "fill_elev", "LABEL": "Fill Elevation", "TYPE": "float", "DEFAULT": 0.5, "MIN": -1.5, "MAX": 1.5, "GROUP": "Lighting"},
        {"NAME": "fill_azim", "LABEL": "Fill Azimuth", "TYPE": "float", "DEFAULT": -2.2, "MIN": -3.14159, "MAX": 3.14159, "GROUP": "Lighting"},
        {"NAME": "fill_color", "LABEL": "Fill Color", "TYPE": "color", "DEFAULT": [0.35, 0.45, 0.7, 1.0], "GROUP": "Lighting"},
        {"NAME": "fill_intensity", "LABEL": "Fill Intensity", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 8.0, "GROUP": "Lighting"},
        {"NAME": "headlight_intensity", "LABEL": "Headlight", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 8.0, "GROUP": "Lighting"},
        {"NAME": "headlight_color", "LABEL": "Headlight Color", "TYPE": "color", "DEFAULT": [1.0, 0.9, 0.75, 1.0], "GROUP": "Lighting"},
        {"NAME": "amb_top", "LABEL": "Sky Ambient", "TYPE": "color", "DEFAULT": [0.45, 0.48, 0.5, 1.0], "GROUP": "Lighting"},
        {"NAME": "amb_bottom", "LABEL": "Ground Ambient", "TYPE": "color", "DEFAULT": [0.12, 0.14, 0.1, 1.0], "GROUP": "Lighting"},
        {"NAME": "ao_strength", "LABEL": "Occlusion", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Lighting"},
        {"NAME": "bounce", "LABEL": "Bounce Light", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "GROUP": "Lighting"},
        {"NAME": "color_source", "LABEL": "Color Source", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Iterations", "Orbit Trap", "Stretch", "Slope", "Part"], "GROUP": "Palette"},
        {"NAME": "palette_offset", "LABEL": "Palette Offset", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Palette"},
        {"NAME": "palette_scale", "LABEL": "Palette Scale", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.05, "MAX": 8.0, "GROUP": "Palette"},
        {"NAME": "color1", "LABEL": "Color 1", "TYPE": "color", "DEFAULT": [0.5, 0.49, 0.46, 1.0], "GROUP": "Palette"},
        {"NAME": "color2", "LABEL": "Color 2", "TYPE": "color", "DEFAULT": [0.62, 0.6, 0.56, 1.0], "GROUP": "Palette"},
        {"NAME": "color3", "LABEL": "Color 3", "TYPE": "color", "DEFAULT": [0.38, 0.37, 0.35, 1.0], "GROUP": "Palette"},
        {"NAME": "color4", "LABEL": "Color 4", "TYPE": "color", "DEFAULT": [0.3, 0.42, 0.25, 1.0], "GROUP": "Palette"},
        {"NAME": "roughness", "LABEL": "Roughness", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.05, "MAX": 1.0, "GROUP": "Palette"},
        {"NAME": "metallic", "LABEL": "Metallic", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Palette"},
        {"NAME": "specular", "LABEL": "Specular", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "GROUP": "Palette"},
        {"NAME": "emit_band", "LABEL": "Glow Band", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0, "GROUP": "Palette"},
        {"NAME": "emit_width", "LABEL": "Glow Width", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.5, "GROUP": "Palette"},
        {"NAME": "emit_gain", "LABEL": "Glow Gain", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 32.0, "GROUP": "Palette"},
        {"NAME": "fog_density", "LABEL": "Fog", "TYPE": "float", "DEFAULT": 0.06, "MIN": 0.0, "MAX": 4.0, "GROUP": "Atmosphere"},
        {"NAME": "fog_color", "LABEL": "Fog Color", "TYPE": "color", "DEFAULT": [0.62, 0.64, 0.66, 1.0], "GROUP": "Atmosphere"},
        {"NAME": "dyn_fog", "LABEL": "Iteration Fog", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0, "GROUP": "Atmosphere"},
        {"NAME": "dyn_fog_color", "LABEL": "Iteration Fog Color", "TYPE": "color", "DEFAULT": [0.9, 0.6, 0.35, 1.0], "GROUP": "Atmosphere"},
        {"NAME": "fog_on_iteration", "LABEL": "Fog On Iteration", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 64.0, "GROUP": "Atmosphere"},
        {"NAME": "sky_brightness", "LABEL": "Sky Brightness", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 4.0, "GROUP": "Atmosphere"},
        {"NAME": "shafts", "LABEL": "Light Shafts", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0, "GROUP": "Atmosphere"},
        {"NAME": "shaft_anisotropy", "LABEL": "Shaft Forward Scatter", "TYPE": "float", "DEFAULT": 0.3, "MIN": -0.9, "MAX": 0.9, "GROUP": "Atmosphere"},
        {"NAME": "dof_mode", "LABEL": "Depth of Field", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2], "LABELS": ["Off", "Focus", "Ramp"], "GROUP": "Lens"},
        {"NAME": "aperture", "LABEL": "Aperture", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "GROUP": "Lens"},
        {"NAME": "autofocus", "LABEL": "Autofocus", "TYPE": "bool", "DEFAULT": true, "GROUP": "Lens"},
        {"NAME": "focus_distance", "LABEL": "Focus Distance", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.01, "MAX": 20.0, "GROUP": "Lens"},
        {"NAME": "exposure", "LABEL": "Exposure", "TYPE": "float", "DEFAULT": 0.6, "MIN": -4.0, "MAX": 4.0, "GROUP": "Grade"},
        {"NAME": "contrast", "LABEL": "Contrast", "TYPE": "float", "DEFAULT": 1.1, "MIN": 0.5, "MAX": 2.0, "GROUP": "Grade"},
        {"NAME": "saturation", "LABEL": "Saturation", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "GROUP": "Grade"},
        {"NAME": "bloom", "LABEL": "Bloom", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 2.0, "GROUP": "Grade"},
        {"NAME": "bloom_threshold", "LABEL": "Bloom Threshold", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 8.0, "GROUP": "Grade"},
        {"NAME": "vignette", "LABEL": "Vignette", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "GROUP": "Grade"},
        {"NAME": "aberration", "LABEL": "Chromatic Aberration", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Grade"},
        {"NAME": "grain", "LABEL": "Grain", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.2, "GROUP": "Grade"},
        {"NAME": "surface", "LABEL": "Surface", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6], "LABELS": ["None", "Stone", "Mossy Rock", "Weathered Metal", "Bark", "Forest Ground", "Ice"], "GROUP": "Surface"},
        {"NAME": "surface_mapping", "LABEL": "Mapping", "TYPE": "long", "DEFAULT": 3, "VALUES": [3, 0, 1, 2], "LABELS": ["World", "Orbit Plane", "Orbit Sphere", "Iterations"], "GROUP": "Surface"},
        {"NAME": "surface_scale", "LABEL": "Scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.05, "MAX": 8.0, "GROUP": "Surface"},
        {"NAME": "surface_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "GROUP": "Surface"},
        {"NAME": "surface_bump", "LABEL": "Bump", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0, "GROUP": "Surface"},
        {"NAME": "surface_depth", "LABEL": "Depth", "TYPE": "float", "DEFAULT": 2.0, "MIN": 1.0, "MAX": 16.0, "GROUP": "Surface"}
    ],
    "IMPORTED": {
        "surfaces": {"PATH": "fractal_textures/surfaces.png"}
    },
    "PREPROCESSORS": [
        {"NAME": "flight", "TYPE": "fractal_flight", "FORMAT": "rgba32float", "OPTIONS": {"bind_all_inputs": true}}
    ],
    "SPECIALIZE_PASSES": true,
    "COLUMNS": [
        {"TITLE": "Formula", "GROUPS": ["Form", "Slot 1", "Slot 2", "Slot 3", "Slot 4", "Slot 5", "Slot 6"]},
        {"TITLE": "Light", "GROUPS": ["Lighting", "Palette", "Surface"]},
        {"TITLE": "Air & Lens", "GROUPS": ["Atmosphere", "Lens", "Grade"]},
        {"TITLE": "Detail", "GROUPS": ["Detail"]}
    ],
    "PASSES": [
        {"TARGET": "tiles", "FORMAT": "r32float", "WIDTH": "$WIDTH*$render_scale/8", "HEIGHT": "$HEIGHT*$render_scale/8"},
        {"TARGETS": ["gA", "gB", "gC"], "FORMATS": ["rgba32float", "rgba16float", "rg32float"], "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGET": "shade", "HISTORY": true, "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGET": "vol", "HISTORY": true, "WIDTH": "$WIDTH*$render_scale/4", "HEIGHT": "$HEIGHT*$render_scale/4"},
        {"TARGET": "hdr", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGETS": ["taa", "taaN"], "FORMATS": ["rgba16float", "r32float"], "HISTORY": true},
        {"TARGET": "sharp"},
        {"TARGET": "dof", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"},
        {"TARGET": "bloom1", "WIDTH": "$WIDTH/4", "HEIGHT": "$HEIGHT/4"},
        {"TARGET": "bloom2", "WIDTH": "$WIDTH/8", "HEIGHT": "$HEIGHT/8"},
        {"TARGET": "bloom3", "WIDTH": "$WIDTH/16", "HEIGHT": "$HEIGHT/16"},
        {}
    ]
}*/

#version 450

layout(location = 0) in vec2 uv;
// Pass 0 writes the G-buffer; the output pass writes out0 only.
layout(location = 0) out vec4 out0;
layout(location = 1) out vec4 out1;
layout(location = 2) out vec4 out2;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
    vec2 JITTER;
    int JITTERINDEX;
    int HISTORYVALID;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
// One eighth resolution: how far every ray of an 8x8 tile can start.
layout(set = 0, binding = 2) uniform texture2D tiles;
// gA: hit distance t, smooth iteration, orbit trap, iteration fog count.
layout(set = 0, binding = 3) uniform texture2D gA;
// gB: normal, log2(dr) (negative for DE-combine part 2).
layout(set = 0, binding = 4) uniform texture2D gB;
// gC: the surface texture's coordinates at the hit, from the orbit. 32-bit:
// they can be large, and half floats step visibly up close.
layout(set = 0, binding = 5) uniform texture2D gC;
// Render size, accumulated: key-light shadow, occlusion, t.
layout(set = 0, binding = 6) uniform texture2D shade;
// Quarter resolution, accumulated: light shaft amount, end distance.
layout(set = 0, binding = 7) uniform texture2D vol;
// Lit HDR color.
layout(set = 0, binding = 8) uniform texture2D hdr;
// Accumulated color and distance.
layout(set = 0, binding = 9) uniform texture2D taa;
// The accumulated sample weight behind each taa pixel.
layout(set = 0, binding = 10) uniform texture2D taaN;
// TAA's result sharpened: what depth of field, bloom and the composite read.
layout(set = 0, binding = 11) uniform texture2D sharp;
// Half resolution: depth-of-field color, circle of confusion.
layout(set = 0, binding = 12) uniform texture2D dof;
// Bloom at 1/4, 1/8 and 1/16 resolution.
layout(set = 0, binding = 13) uniform texture2D bloom1;
layout(set = 0, binding = 14) uniform texture2D bloom2;
layout(set = 0, binding = 15) uniform texture2D bloom3;
// Camera and formula schedule from the fractal_flight preprocessor.
// Imported: six ambientCG CC0 materials, 3 x 2; RGB color, A height.
layout(set = 0, binding = 16) uniform texture2D surfaces;
layout(set = 0, binding = 17) uniform texture2D flight;

const int PASS_TILES = 0;
const int PASS_GBUFFER = 1;
const int PASS_SHADE = 2;
const int PASS_VOLUME = 3;
const int PASS_LIGHT = 4;
const int PASS_TAA = 5;
const int PASS_SHARP = 6;
const int PASS_DOF = 7;
const int PASS_BLOOM1 = 8;
const int PASS_BLOOM2 = 9;
const int PASS_BLOOM3 = 10;
const int TILE = 8;

layout(set = 0, binding = 18) uniform UserParams {
    float throttle;
    float speed;
    int flight_mode;
    float look_heading;
    float look_pitch;
    float look_roll;
    float yaw_rate;
    float pitch_rate;
    float roll_rate;
    float strafe_x;
    float strafe_y;
    float fov;
    uint reset_camera;
    uint find_inside;
    float autopilot;
    uint save_location;
    float location;
    float recall_time;
    uint tour;
    float tour_seconds;
    int hybrid_mode;
    int repeat_from;
    float max_iterations;
    float bailout;
    uint julia_mode;
    float julia_x;
    float julia_y;
    float julia_z;
    int combine_split;
    int combine_op;
    float combine_width;
    int slot1_formula;
    int slot1_mode;
    float slot1_count;
    float slot1_a;
    float slot1_b;
    float slot1_c;
    float slot1_d;
    float slot1_rot_x;
    float slot1_rot_y;
    float slot1_rot_z;
    int slot2_formula;
    int slot2_mode;
    float slot2_count;
    float slot2_a;
    float slot2_b;
    float slot2_c;
    float slot2_d;
    float slot2_rot_x;
    float slot2_rot_y;
    float slot2_rot_z;
    int slot3_formula;
    int slot3_mode;
    float slot3_count;
    float slot3_a;
    float slot3_b;
    float slot3_c;
    float slot3_d;
    float slot3_rot_x;
    float slot3_rot_y;
    float slot3_rot_z;
    int slot4_formula;
    int slot4_mode;
    float slot4_count;
    float slot4_a;
    float slot4_b;
    float slot4_c;
    float slot4_d;
    float slot4_rot_x;
    float slot4_rot_y;
    float slot4_rot_z;
    int slot5_formula;
    int slot5_mode;
    float slot5_count;
    float slot5_a;
    float slot5_b;
    float slot5_c;
    float slot5_d;
    float slot5_rot_x;
    float slot5_rot_y;
    float slot5_rot_z;
    int slot6_formula;
    int slot6_mode;
    float slot6_count;
    float slot6_a;
    float slot6_b;
    float slot6_c;
    float slot6_d;
    float slot6_rot_x;
    float slot6_rot_y;
    float slot6_rot_z;
    float detail;
    float lod;
    float max_steps;
    float step_mult;
    uint temporal;
    uint tile_prepass;
    float render_scale;
    float target_fps;
    float still_samples;
    float sharpness;
    int debug_view;
    int look;
    float sun_elev;
    float sun_azim;
    vec4 sun_color;
    float sun_intensity;
    float shadow_softness;
    float shadow_length;
    float fill_elev;
    float fill_azim;
    vec4 fill_color;
    float fill_intensity;
    float headlight_intensity;
    vec4 headlight_color;
    vec4 amb_top;
    vec4 amb_bottom;
    float ao_strength;
    float bounce;
    int color_source;
    float palette_offset;
    float palette_scale;
    vec4 color1;
    vec4 color2;
    vec4 color3;
    vec4 color4;
    float roughness;
    float metallic;
    float specular;
    float emit_band;
    float emit_width;
    float emit_gain;
    float fog_density;
    vec4 fog_color;
    float dyn_fog;
    vec4 dyn_fog_color;
    float fog_on_iteration;
    float sky_brightness;
    float shafts;
    float shaft_anisotropy;
    int dof_mode;
    float aperture;
    uint autofocus;
    float focus_distance;
    float exposure;
    float contrast;
    float saturation;
    float bloom;
    float bloom_threshold;
    float vignette;
    float aberration;
    float grain;
    int surface;
    int surface_mapping;
    float surface_scale;
    float surface_amount;
    float surface_bump;
    float surface_depth;
};

// ── Look presets ─────────────────────────────────────────────────────────

// The look inputs as this pass uses them: the sliders under Custom, or one
// of the looks below. Colors are linear, as the engine uploads the sliders'
// sRGB values. applyLook() fills them at the start of every pass; the
// defines after it point the rest of the shader here.
float look_sun_elev;
float look_sun_azim;
vec4 look_sun_color;
float look_sun_intensity;
float look_fill_elev;
float look_fill_azim;
vec4 look_fill_color;
float look_fill_intensity;
float look_headlight_intensity;
vec4 look_headlight_color;
vec4 look_amb_top;
vec4 look_amb_bottom;
float look_ao_strength;
float look_bounce;
float look_palette_offset;
float look_palette_scale;
vec4 look_color1;
vec4 look_color2;
vec4 look_color3;
vec4 look_color4;
float look_roughness;
float look_metallic;
float look_specular;
float look_fog_density;
vec4 look_fog_color;
float look_dyn_fog;
vec4 look_dyn_fog_color;
float look_sky_brightness;
float look_shafts;
float look_shaft_anisotropy;
float look_exposure;
float look_contrast;
float look_saturation;
float look_bloom;

void applyLook() {
    look_sun_elev = sun_elev;
    look_sun_azim = sun_azim;
    look_sun_color = sun_color;
    look_sun_intensity = sun_intensity;
    look_fill_elev = fill_elev;
    look_fill_azim = fill_azim;
    look_fill_color = fill_color;
    look_fill_intensity = fill_intensity;
    look_headlight_intensity = headlight_intensity;
    look_headlight_color = headlight_color;
    look_amb_top = amb_top;
    look_amb_bottom = amb_bottom;
    look_ao_strength = ao_strength;
    look_bounce = bounce;
    look_palette_offset = palette_offset;
    look_palette_scale = palette_scale;
    look_color1 = color1;
    look_color2 = color2;
    look_color3 = color3;
    look_color4 = color4;
    look_roughness = roughness;
    look_metallic = metallic;
    look_specular = specular;
    look_fog_density = fog_density;
    look_fog_color = fog_color;
    look_dyn_fog = dyn_fog;
    look_dyn_fog_color = dyn_fog_color;
    look_sky_brightness = sky_brightness;
    look_shafts = shafts;
    look_shaft_anisotropy = shaft_anisotropy;
    look_exposure = exposure;
    look_contrast = contrast;
    look_saturation = saturation;
    look_bloom = bloom;
    if (look == 1) {
        // Stone Hall.
        look_sun_elev = 0.6;
        look_sun_azim = 2.4;
        look_sun_color = vec4(1.0, 0.8276, 0.6038, 1.0);
        look_sun_intensity = 4.0;
        look_fill_elev = 0.5;
        look_fill_azim = -2.2;
        look_fill_color = vec4(0.1005, 0.1706, 0.448, 1.0);
        look_fill_intensity = 0.6;
        look_headlight_intensity = 0.3;
        look_headlight_color = vec4(1.0, 0.7874, 0.5225, 1.0);
        look_amb_top = vec4(0.1706, 0.196, 0.214, 1.0);
        look_amb_bottom = vec4(0.0134, 0.0174, 0.01, 1.0);
        look_ao_strength = 1.0;
        look_bounce = 0.3;
        look_palette_offset = 0.0;
        look_palette_scale = 0.3;
        look_color1 = vec4(0.214, 0.2049, 0.1789, 1.0);
        look_color2 = vec4(0.3424, 0.3185, 0.2738, 1.0);
        look_color3 = vec4(0.1193, 0.1128, 0.1005, 1.0);
        look_color4 = vec4(0.0732, 0.1473, 0.0509, 1.0);
        look_roughness = 0.7;
        look_metallic = 0.0;
        look_specular = 0.5;
        look_fog_density = 0.06;
        look_fog_color = vec4(0.3424, 0.3672, 0.3931, 1.0);
        look_dyn_fog = 0.0;
        look_dyn_fog_color = vec4(0.7874, 0.3185, 0.1005, 1.0);
        look_sky_brightness = 0.8;
        look_shafts = 0.0;
        look_shaft_anisotropy = 0.3;
        look_exposure = 0.6;
        look_contrast = 1.1;
        look_saturation = 1.0;
        look_bloom = 0.15;
    } else if (look == 2) {
        // Desert Sunbeams.
        look_sun_elev = 0.3;
        look_sun_azim = 1.2;
        look_sun_color = vec4(1.0, 0.5225, 0.1706, 1.0);
        look_sun_intensity = 8.0;
        look_fill_elev = 0.5;
        look_fill_azim = -2.2;
        look_fill_color = vec4(0.1005, 0.1706, 0.448, 1.0);
        look_fill_intensity = 0.4;
        look_headlight_intensity = 0.2;
        look_headlight_color = vec4(1.0, 0.7874, 0.5225, 1.0);
        look_amb_top = vec4(0.0331, 0.0637, 0.1193, 1.0);
        look_amb_bottom = vec4(0.01, 0.006, 0.0031, 1.0);
        look_ao_strength = 0.8;
        look_bounce = 0.5;
        look_palette_offset = 0.0;
        look_palette_scale = 0.3;
        look_color1 = vec4(0.1473, 0.1065, 0.0732, 1.0);
        look_color2 = vec4(0.2633, 0.196, 0.1193, 1.0);
        look_color3 = vec4(0.0732, 0.0593, 0.047, 1.0);
        look_color4 = vec4(0.3424, 0.1193, 0.0397, 1.0);
        look_roughness = 0.6;
        look_metallic = 0.0;
        look_specular = 0.6;
        look_fog_density = 0.08;
        look_fog_color = vec4(0.1473, 0.1065, 0.0732, 1.0);
        look_dyn_fog = 0.0;
        look_dyn_fog_color = vec4(0.7874, 0.3185, 0.1005, 1.0);
        look_sky_brightness = 0.8;
        look_shafts = 0.8;
        look_shaft_anisotropy = 0.5;
        look_exposure = 0.8;
        look_contrast = 1.15;
        look_saturation = 1.05;
        look_bloom = 0.25;
    } else if (look == 3) {
        // Moonlit.
        look_sun_elev = 0.35;
        look_sun_azim = -2.6;
        look_sun_color = vec4(0.3801, 0.5705, 1.0, 1.0);
        look_sun_intensity = 9.0;
        look_fill_elev = 0.5;
        look_fill_azim = -2.2;
        look_fill_color = vec4(0.1005, 0.1706, 0.448, 1.0);
        look_fill_intensity = 0.3;
        look_headlight_intensity = 0.4;
        look_headlight_color = vec4(1.0, 0.6038, 0.2633, 1.0);
        look_amb_top = vec4(0.01, 0.0174, 0.0331, 1.0);
        look_amb_bottom = vec4(0.0023, 0.0023, 0.0031, 1.0);
        look_ao_strength = 0.9;
        look_bounce = 0.2;
        look_palette_offset = 0.0;
        look_palette_scale = 0.3;
        look_color1 = vec4(0.0732, 0.0835, 0.0946, 1.0);
        look_color2 = vec4(0.1706, 0.1873, 0.196, 1.0);
        look_color3 = vec4(0.0331, 0.0397, 0.047, 1.0);
        look_color4 = vec4(0.2633, 0.2846, 0.2957, 1.0);
        look_roughness = 0.5;
        look_metallic = 0.1;
        look_specular = 0.8;
        look_fog_density = 0.1;
        look_fog_color = vec4(0.01, 0.0174, 0.0397, 1.0);
        look_dyn_fog = 0.0;
        look_dyn_fog_color = vec4(0.7874, 0.3185, 0.1005, 1.0);
        look_sky_brightness = 0.6;
        look_shafts = 0.0;
        look_shaft_anisotropy = 0.3;
        look_exposure = 1.6;
        look_contrast = 1.15;
        look_saturation = 0.9;
        look_bloom = 0.2;
    } else if (look == 4) {
        // Teal & Gold.
        look_sun_elev = 0.25;
        look_sun_azim = 2.4;
        look_sun_color = vec4(1.0, 0.6921, 0.3185, 1.0);
        look_sun_intensity = 5.0;
        look_fill_elev = 0.5;
        look_fill_azim = -2.2;
        look_fill_color = vec4(0.1005, 0.1706, 0.448, 1.0);
        look_fill_intensity = 0.8;
        look_headlight_intensity = 0.3;
        look_headlight_color = vec4(1.0, 0.7874, 0.5225, 1.0);
        look_amb_top = vec4(0.0272, 0.1193, 0.1706, 1.0);
        look_amb_bottom = vec4(0.0031, 0.0049, 0.006, 1.0);
        look_ao_strength = 0.8;
        look_bounce = 0.3;
        look_palette_offset = 0.0;
        look_palette_scale = 0.3;
        look_color1 = vec4(0.0031, 0.0397, 0.0593, 1.0);
        look_color2 = vec4(0.6921, 0.3424, 0.0732, 1.0);
        look_color3 = vec4(0.0015, 0.0085, 0.0116, 1.0);
        look_color4 = vec4(0.448, 0.6038, 0.5705, 1.0);
        look_roughness = 0.35;
        look_metallic = 0.4;
        look_specular = 1.2;
        look_fog_density = 0.15;
        look_fog_color = vec4(0.01, 0.0397, 0.0593, 1.0);
        look_dyn_fog = 0.0;
        look_dyn_fog_color = vec4(0.7874, 0.3185, 0.1005, 1.0);
        look_sky_brightness = 0.8;
        look_shafts = 0.0;
        look_shaft_anisotropy = 0.3;
        look_exposure = 0.7;
        look_contrast = 1.1;
        look_saturation = 1.0;
        look_bloom = 0.15;
    }
}

#define sun_elev look_sun_elev
#define sun_azim look_sun_azim
#define sun_color look_sun_color
#define sun_intensity look_sun_intensity
#define fill_elev look_fill_elev
#define fill_azim look_fill_azim
#define fill_color look_fill_color
#define fill_intensity look_fill_intensity
#define headlight_intensity look_headlight_intensity
#define headlight_color look_headlight_color
#define amb_top look_amb_top
#define amb_bottom look_amb_bottom
#define ao_strength look_ao_strength
#define bounce look_bounce
#define palette_offset look_palette_offset
#define palette_scale look_palette_scale
#define color1 look_color1
#define color2 look_color2
#define color3 look_color3
#define color4 look_color4
#define roughness look_roughness
#define metallic look_metallic
#define specular look_specular
#define fog_density look_fog_density
#define fog_color look_fog_color
#define dyn_fog look_dyn_fog
#define dyn_fog_color look_dyn_fog_color
#define sky_brightness look_sky_brightness
#define shafts look_shafts
#define shaft_anisotropy look_shaft_anisotropy
#define exposure look_exposure
#define contrast look_contrast
#define saturation look_saturation
#define bloom look_bloom

// The formula stack's structure, as specialization constants set from the
// SPECIALIZE inputs of the same name. The compiler keeps only the formulas
// and slots the current stack uses.
//
// Read them through specialized(): naga cannot parse the SPIR-V glslang
// emits for arithmetic on a specialization constant, and a function call
// keeps that arithmetic out of constant expressions. The value is still a
// constant once the pipeline is built, so the GPU compiler folds it.
int specialized(int value) {
    return value;
}

layout(constant_id = 0) const int SPEC_HYBRID_MODE = 0;
#define SC_HYBRID_MODE specialized(SPEC_HYBRID_MODE)
layout(constant_id = 1) const int SPEC_REPEAT_FROM = 1;
#define SC_REPEAT_FROM specialized(SPEC_REPEAT_FROM)
layout(constant_id = 2) const int SPEC_COMBINE_SPLIT = 4;
#define SC_COMBINE_SPLIT specialized(SPEC_COMBINE_SPLIT)
layout(constant_id = 3) const int SPEC_SLOT1_FORMULA = 1;
#define SC_SLOT1_FORMULA specialized(SPEC_SLOT1_FORMULA)
layout(constant_id = 4) const int SPEC_SLOT1_MODE = 0;
#define SC_SLOT1_MODE specialized(SPEC_SLOT1_MODE)
layout(constant_id = 5) const int SPEC_SLOT1_COUNT = 1;
#define SC_SLOT1_COUNT specialized(SPEC_SLOT1_COUNT)
layout(constant_id = 6) const int SPEC_SLOT2_FORMULA = 0;
#define SC_SLOT2_FORMULA specialized(SPEC_SLOT2_FORMULA)
layout(constant_id = 7) const int SPEC_SLOT2_MODE = 0;
#define SC_SLOT2_MODE specialized(SPEC_SLOT2_MODE)
layout(constant_id = 8) const int SPEC_SLOT2_COUNT = 1;
#define SC_SLOT2_COUNT specialized(SPEC_SLOT2_COUNT)
layout(constant_id = 9) const int SPEC_SLOT3_FORMULA = 0;
#define SC_SLOT3_FORMULA specialized(SPEC_SLOT3_FORMULA)
layout(constant_id = 10) const int SPEC_SLOT3_MODE = 0;
#define SC_SLOT3_MODE specialized(SPEC_SLOT3_MODE)
layout(constant_id = 11) const int SPEC_SLOT3_COUNT = 1;
#define SC_SLOT3_COUNT specialized(SPEC_SLOT3_COUNT)
layout(constant_id = 12) const int SPEC_SLOT4_FORMULA = 0;
#define SC_SLOT4_FORMULA specialized(SPEC_SLOT4_FORMULA)
layout(constant_id = 13) const int SPEC_SLOT4_MODE = 0;
#define SC_SLOT4_MODE specialized(SPEC_SLOT4_MODE)
layout(constant_id = 14) const int SPEC_SLOT4_COUNT = 1;
#define SC_SLOT4_COUNT specialized(SPEC_SLOT4_COUNT)
layout(constant_id = 15) const int SPEC_SLOT5_FORMULA = 0;
#define SC_SLOT5_FORMULA specialized(SPEC_SLOT5_FORMULA)
layout(constant_id = 16) const int SPEC_SLOT5_MODE = 0;
#define SC_SLOT5_MODE specialized(SPEC_SLOT5_MODE)
layout(constant_id = 17) const int SPEC_SLOT5_COUNT = 1;
#define SC_SLOT5_COUNT specialized(SPEC_SLOT5_COUNT)
layout(constant_id = 18) const int SPEC_SLOT6_FORMULA = 0;
#define SC_SLOT6_FORMULA specialized(SPEC_SLOT6_FORMULA)
layout(constant_id = 19) const int SPEC_SLOT6_MODE = 0;
#define SC_SLOT6_MODE specialized(SPEC_SLOT6_MODE)
layout(constant_id = 20) const int SPEC_SLOT6_COUNT = 1;
#define SC_SLOT6_COUNT specialized(SPEC_SLOT6_COUNT)

// This pipeline's pass (SPECIALIZE_PASSES), so each pass compiles alone and
// runs with the registers it needs, not those of the largest pass.
layout(constant_id = 21) const int SPEC_PASSINDEX = 0;
#define PASS specialized(SPEC_PASSINDEX)

// ── Flight texture ───────────────────────────────────────────────────────

const float FLIGHT_LAYOUT = 6.0;
const int ROTATION_TEXEL = 11;
const int KIND_TEXEL = 29;
const int RESOLUTION_TEXEL = 30;

vec4 flightTexel(int i) {
    return texelFetch(sampler2D(flight, texSampler), ivec2(i, 0), 0);
}

bool flightValid() {
    return flightTexel(10).z == FLIGHT_LAYOUT;
}

// The length fog, shadows and the headlight are measured in: the flight's
// scene scale. While walking it stays fixed, so fog belongs to the place and
// does not thicken as the camera nears a wall; while diving it follows the
// distance to the surface, so the look holds at any depth.
float sceneScale() {
    return max(flightTexel(10).w, 1e-6);
}

// Width over height of the deck's output.
float outputAspect() {
    vec2 size = vec2(textureSize(sampler2D(taa, texSampler), 0));
    return size.x / size.y;
}

// The render-size targets as allocated, at Max Render Scale.
vec2 allocatedSize() {
    return vec2(textureSize(sampler2D(gA, texSampler), 0));
}

vec2 liveSize(float scale) {
    vec2 alloc = allocatedSize();
    return clamp(floor(vec2(textureSize(sampler2D(taa, texSampler), 0)) * scale + 0.5), vec2(1.0), alloc);
}

// The size the march and lighting run at this frame: the output size times
// the live render scale the flight picked to hold Target FPS. It is the top
// left of the allocated targets; reduced-size passes must not use
// RENDERSIZE for this.
vec2 frameSize() {
    return liveSize(flightTexel(RESOLUTION_TEXEL).x);
}

// Last frame's frameSize(), where the history of shade and volume lies.
vec2 previousFrameSize() {
    return liveSize(flightTexel(RESOLUTION_TEXEL).y);
}

// Whether this fragment of a pass at 1/divisor of render size lies outside
// the part drawn this frame.
bool outsideLive(float divisor) {
    return any(greaterThanEqual(gl_FragCoord.xy, ceil(frameSize() / divisor)));
}

// Where a point of the live frame, in render pixels, lies in a texture at
// 1/divisor of render size, as a normalized coordinate kept half a texel
// inside the drawn part so bilinear taps do not reach undrawn texels.
vec2 liveUv(vec2 renderPixel, vec2 liveRender, float divisor, vec2 textureSize_) {
    vec2 texel = renderPixel / divisor;
    vec2 drawn = ceil(liveRender / divisor);
    return clamp(texel, vec2(0.5), drawn - 0.5) / textureSize_;
}

// The lit render image at a normalized output coordinate, bilinear.
vec3 hdrAtOutput(vec2 outputUv) {
    vec2 live = frameSize();
    return texture(sampler2D(hdr, texSampler), liveUv(outputUv * live, live, 1.0, allocatedSize())).rgb;
}

// The deck's size, which TAA, depth of field, bloom and the composite fill.
vec2 outputSize() {
    return vec2(textureSize(sampler2D(taa, texSampler), 0));
}

// The render texel an output texel falls in.
ivec2 toRender(ivec2 outputTexel) {
    vec2 render = frameSize();
    ivec2 at = ivec2((vec2(outputTexel) + 0.5) * render / outputSize());
    return clamp(at, ivec2(0), ivec2(render) - 1);
}

// A pixel's sample position, offset by this frame's jitter when TAA is on.
vec2 jittered(vec2 frag) {
    return temporal != 0u ? frag + JITTER : frag;
}

// A slot's rotation, built on the host. `on` is false when unrotated.
struct Rot {
    bool on;
    vec3 r0;
    vec3 r1;
    vec3 r2;
};

vec3 rotateBy(Rot r, vec3 v) {
    if (!r.on) {
        return v;
    }
    return vec3(dot(r.r0, v), dot(r.r1, v), dot(r.r2, v));
}

// Jacobian columns: how z moves per unit move of the sample point along x,
// y and z. Linear steps apply to each column as they apply to z.
mat3 rotateColumns(Rot r, mat3 m) {
    return mat3(rotateBy(r, m[0]), rotateBy(r, m[1]), rotateBy(r, m[2]));
}

// The column vector c times the row vector r.
mat3 columnTimesRow(vec3 c, vec3 r) {
    return mat3(c * r.x, c * r.y, c * r.z);
}

mat3 timesEach(mat3 m, vec3 s) {
    return mat3(m[0] * s, m[1] * s, m[2] * s);
}

// The derivative of scaling by m ~ 1 / rr, rr = q . q, without the m.
vec3 radialColumn(vec3 c, vec3 z, vec3 q, float rr) {
    return c - z * (2.0 * dot(q, c) / rr);
}

mat3 radialColumns(mat3 m, vec3 z, vec3 q, float rr) {
    return mat3(radialColumn(m[0], z, q, rr), radialColumn(m[1], z, q, rr), radialColumn(m[2], z, q, rr));
}

// The slope of abs(): never 0, which would drop a Jacobian row where a
// component lands exactly on 0.
vec3 signs(vec3 v) {
    return mix(vec3(-1.0), vec3(1.0), greaterThanEqual(v, vec3(0.0)));
}

float signs(float v) {
    return v >= 0.0 ? 1.0 : -1.0;
}

// 1 where clamp(a, -limit, limit) * 2 - a passes a through, -1 where it reflects.
vec3 foldSigns(vec3 v, float limit) {
    return mix(vec3(-1.0), vec3(1.0), lessThanEqual(abs(v), vec3(limit)));
}

float frobenius(mat3 m) {
    return sqrt(dot(m[0], m[0]) + dot(m[1], m[1]) + dot(m[2], m[2]));
}

// ── Formulas (same sequences as src/internal/fractal/formula.rs) ──────────

const float MIN_RADIUS2 = 1e-8;
const float PHI = 1.618033988749895;

float planeReflect(inout vec3 z, vec3 n, float intensity, inout mat3 J, bool jac) {
    float h = dot(z, n);
    if (h < 0.0) {
        z -= n * (2.0 * h * intensity);
        if (jac) {
            J = J - columnTimesRow(n, (2.0 * intensity) * (n * J));
        }
        return max(abs(1.0 - 2.0 * intensity), 1.0);
    }
    return 1.0;
}

bool swapIfLess(inout float a, inout float b) {
    if (a < b) {
        float t = a;
        a = b;
        b = t;
        return true;
    }
    return false;
}

bool reflectPair(inout float a, inout float b) {
    if (a + b < 0.0) {
        float na = -b;
        b = -a;
        a = na;
        return true;
    }
    return false;
}

// Components i and j of every column exchanged, negated when `negate`.
mat3 exchangeRows(mat3 m, int i, int j, bool negate) {
    float s = negate ? -1.0 : 1.0;
    for (int k = 0; k < 3; k++) {
        float a = m[k][i];
        m[k][i] = s * m[k][j];
        m[k][j] = s * a;
    }
    return m;
}

float gnarlAxis(float x, float from, float step, float alpha, float beta) {
    return x - step * sin(from + sin(alpha * (from + sin(beta * from))));
}

float gnarlSlope(float from, float step, float alpha, float beta) {
    float inner = alpha * (from + sin(beta * from));
    return step * cos(from + sin(inner)) * (1.0 + cos(inner) * alpha * (1.0 + beta * cos(beta * from)));
}

// One axis of Repeat: the position within its cell, the cell clamped to
// `count` copies each way when positive, odd cells mirrored.
float repeatCell(float a, float size, float count) {
    float cell = floor(a / size + 0.5);
    if (count >= 1.0) {
        cell = clamp(cell, -floor(count), floor(count));
    }
    return cell;
}

// -1 in a mirrored cell, else 1: the derivative of repeatAxis.
float repeatSign(float a, float size, float count, bool mirror) {
    return (mirror && mod(repeatCell(a, size, count), 2.0) == 1.0) ? -1.0 : 1.0;
}

float repeatAxis(float a, float size, float count, bool mirror) {
    return (a - repeatCell(a, size, count) * size) * repeatSign(a, size, count, mirror);
}

// Repeat's warp: sines with wavelengths in irrational ratio (in cells,
// 2 phi and 2 (1 + sqrt 2)), so the bend never repeats and every cell
// differs. Returns the stretch bound in `stretch`.
vec3 quasiWarp(vec3 v, float size, float amount, float phase, out float stretch) {
    if (amount == 0.0) {
        stretch = 1.0;
        return v;
    }
    float a = 0.5 * amount * size;
    float k1 = 6.283185307179586 / (size * 2.0 * PHI);
    float k2 = 6.283185307179586 / (size * 2.0 * 2.414213562373095);
    float p2 = 1.7 * phase;
    stretch = 1.0 + 2.0 * abs(a) * (k1 + k2);
    return v + a * vec3(
        sin(k1 * v.y + phase) + sin(k2 * v.z + p2),
        sin(k1 * v.z + 2.3 + phase) + sin(k2 * v.x + 0.6 + p2),
        sin(k1 * v.x + 4.1 + phase) + sin(k2 * v.y + 3.3 + p2));
}

vec3 repeatCells(vec3 v, float size, float count, bool mirror) {
    float s = max(abs(size), 1e-6);
    return vec3(repeatAxis(v.x, s, count, mirror), repeatAxis(v.y, s, count, mirror), repeatAxis(v.z, s, count, mirror));
}

// The Jacobian of the warp at v, then of the tiling after it, applied to m.
mat3 repeatColumns(mat3 m, vec3 v, float size, float count, bool mirror, float amount, float phase) {
    float s = max(abs(size), 1e-6);
    mat3 warp = mat3(1.0);
    if (amount != 0.0) {
        float a = 0.5 * amount * size;
        float k1 = 6.283185307179586 / (size * 2.0 * PHI);
        float k2 = 6.283185307179586 / (size * 2.0 * 2.414213562373095);
        float p2 = 1.7 * phase;
        // Columns: the change of the moved point per unit of x, y, z.
        warp = mat3(
            1.0, a * k2 * cos(k2 * v.x + 0.6 + p2), a * k1 * cos(k1 * v.x + 4.1 + phase),
            a * k1 * cos(k1 * v.y + phase), 1.0, a * k2 * cos(k2 * v.y + 3.3 + p2),
            a * k2 * cos(k2 * v.z + p2), a * k1 * cos(k1 * v.z + 2.3 + phase), 1.0);
    }
    float unused;
    vec3 w = quasiWarp(v, size, amount, phase, unused);
    vec3 mirrors = vec3(repeatSign(w.x, s, count, mirror), repeatSign(w.y, s, count, mirror), repeatSign(w.z, s, count, mirror));
    return timesEach(warp * m, mirrors);
}

// Rows of the Jacobian of Bulbox's power -2 map at the scaled point v, as
// columns of the returned matrix transposed.
mat3 bulboxPowerJacobian(vec3 v, float rxy, float flatten, float q) {
    float p = rxy * rxy;
    vec3 gradFlatten = vec3(2.0 * v.x * v.z * v.z / (p * p), 2.0 * v.y * v.z * v.z / (p * p), -2.0 * v.z / p);
    vec3 gradQ = v * (-4.0 * q / dot(v, v));
    vec3 gradProduct = gradFlatten * q + gradQ * flatten;
    vec3 xy = vec3(v.x, v.y, 0.0) / rxy;
    vec3 row0 = vec3(2.0 * v.x, -2.0 * v.y, 0.0) * (flatten * q) + gradProduct * (v.x * v.x - v.y * v.y);
    vec3 row1 = vec3(-v.y, -v.x, 0.0) * (flatten * q) - gradProduct * (v.x * v.y);
    vec3 row2 = -2.0 * (vec3(0.0, 0.0, rxy * q) + xy * (v.z * q) + gradQ * (v.z * rxy));
    return transpose(mat3(row0, row1, row2));
}

// One formula step. Called with constant `f` and `mode`, so each call
// compiles to that one formula. When `jac`, J carries the Jacobian of z (see
// src/internal/fractal/formula.rs), and when `jacSeed` S carries the seed's;
// otherwise S stays a multiple of the identity. Mandelbulb leaves J alone,
// since its chains use the power DE.
void applyFormula(int f, int mode, vec4 p, Rot rot, inout vec3 z, inout float dr, inout vec3 seed,
                  inout float seedDr, inout mat3 J, inout mat3 S, bool jac, bool jacSeed,
                  inout float w, inout vec3 Jw) {
    if (f == 1) {
        // Box: scale, min radius, fold limit, fixed radius.
        float fold = p.z;
        bool surf = mode == 1 || mode == 2;
        vec3 folds;
        if (surf) {
            folds = vec3(foldSigns(z, fold).xy, 1.0);
            z.xy = clamp(z.xy, -fold, fold) * 2.0 - z.xy;
        } else if (mode == 3) {
            folds = -signs(z);
            z = vec3(fold) - abs(z);
        } else if (mode == 4) {
            folds = signs(z);
            z = abs(z) + vec3(fold);
        } else {
            folds = foldSigns(z, fold);
            z = clamp(z, -fold, fold) * 2.0 - z;
        }
        vec3 q = mode == 2 ? vec3(z.xy, 0.0) : z;
        float rr = dot(q, q);
        float min2 = max(p.y * p.y, MIN_RADIUS2);
        float fixed2 = max(p.w * p.w, min2);
        float m = p.x * fixed2 / clamp(rr, min2, fixed2);
        vec3 sd = surf ? seed.yxz : seed;
        if (jac) {
            mat3 d = timesEach(J, folds);
            if (rr > min2 && rr < fixed2) {
                d = radialColumns(d, z, q, rr);
            }
            mat3 sd = S;
            if (surf) {
                sd = exchangeRows(S, 0, 1, false);
            }
            J = rotateColumns(rot, d * m + sd);
        }
        z = rotateBy(rot, z * m + sd);
        dr = dr * abs(m) + seedDr;
    } else if (f == 2) {
        // Menger: scale, center offset.
        float scale = p.x;
        float offset = p.y;
        if (jac) {
            J = timesEach(J, signs(z));
        }
        z = abs(z);
        if (swapIfLess(z.x, z.y) && jac) {
            J = exchangeRows(J, 0, 1, false);
        }
        if (swapIfLess(z.x, z.z) && jac) {
            J = exchangeRows(J, 0, 2, false);
        }
        if (swapIfLess(z.y, z.z) && jac) {
            J = exchangeRows(J, 1, 2, false);
        }
        z = rotateBy(rot, z);
        float h = 0.5 * offset * (scale - 1.0) / scale;
        if (jac) {
            J = timesEach(rotateColumns(rot, J), vec3(scale, scale, -signs(z.z - h) * scale));
        }
        z.z = h - abs(z.z - h);
        z.x = scale * z.x - offset * (scale - 1.0);
        z.y = scale * z.y - offset * (scale - 1.0);
        z.z *= scale;
        dr *= abs(scale);
    } else if (f == 3) {
        // Sierpinski: scale, offset.
        if (reflectPair(z.x, z.y) && jac) {
            J = exchangeRows(J, 0, 1, true);
        }
        if (reflectPair(z.x, z.z) && jac) {
            J = exchangeRows(J, 0, 2, true);
        }
        if (reflectPair(z.y, z.z) && jac) {
            J = exchangeRows(J, 1, 2, true);
        }
        if (jac) {
            J = rotateColumns(rot, J) * p.x;
        }
        z = rotateBy(rot, z);
        z = z * p.x - vec3(p.y * (p.x - 1.0));
        dr *= abs(p.x);
    } else if (f == 4) {
        // KIFS: scale, offset, abs first, fold intensity.
        float stretch = 1.0;
        if (mode == 1) {
            if (jac) {
                J = timesEach(J, signs(z));
            }
            z = abs(z);
            stretch *= planeReflect(z, normalize(vec3(1.0, -1.0, 0.0)), p.w, J, jac);
            stretch *= planeReflect(z, normalize(vec3(1.0, 0.0, -1.0)), p.w, J, jac);
            stretch *= planeReflect(z, normalize(vec3(0.0, 1.0, -1.0)), p.w, J, jac);
        } else if (mode == 2) {
            if (jac) {
                J = timesEach(J, signs(z));
            }
            z = abs(z);
            vec3 n1 = normalize(vec3(-1.0, PHI - 1.0, 1.0 / (PHI - 1.0)));
            vec3 n2 = normalize(vec3(PHI - 1.0, 1.0 / (PHI - 1.0), -1.0));
            vec3 n3 = normalize(vec3(1.0 / (PHI - 1.0), -1.0, PHI - 1.0));
            stretch *= planeReflect(z, n1, p.w, J, jac);
            stretch *= planeReflect(z, n2, p.w, J, jac);
            stretch *= planeReflect(z, n3, p.w, J, jac);
            stretch *= planeReflect(z, n2, p.w, J, jac);
        } else {
            if (p.z > 0.5) {
                if (jac) {
                    J = timesEach(J, signs(z));
                }
                z = abs(z);
            }
            stretch *= planeReflect(z, normalize(vec3(1.0, 1.0, 0.0)), p.w, J, jac);
            stretch *= planeReflect(z, normalize(vec3(1.0, 0.0, 1.0)), p.w, J, jac);
            stretch *= planeReflect(z, normalize(vec3(0.0, 1.0, 1.0)), p.w, J, jac);
        }
        if (jac) {
            J = rotateColumns(rot, J) * p.x;
        }
        z = rotateBy(rot, z);
        z = z * p.x - vec3(p.y * (p.x - 1.0));
        dr *= abs(p.x) * stretch;
    } else if (f == 5) {
        // Pseudo-Kleinian: box size, inversion size.
        vec3 folds = foldSigns(z, p.x);
        z = clamp(z, -p.x, p.x) * 2.0 - z;
        float rr = dot(z, z);
        float k = max(p.y / max(rr, MIN_RADIUS2), 1.0);
        if (jac) {
            mat3 d = timesEach(J, folds);
            if (rr > MIN_RADIUS2 && p.y > rr) {
                d = radialColumns(d, z, z, rr);
            }
            J = rotateColumns(rot, d * k);
        }
        z = rotateBy(rot, z * k);
        dr *= k;
    } else if (f == 6) {
        // Kaliset: scale, radius offset.
        vec3 slopes = signs(z);
        z = abs(z);
        float denominator = dot(z, z) + p.y;
        float m = p.x / max(denominator, MIN_RADIUS2);
        if (jac) {
            mat3 d = timesEach(J, slopes);
            if (denominator > MIN_RADIUS2) {
                d = radialColumns(d, z, z, denominator);
            }
            J = rotateColumns(rot, d * m + S);
        }
        z = rotateBy(rot, z * m + seed);
        dr = dr * abs(m) + seedDr;
    } else if (f == 7) {
        // Mandelbulb: power, z multiplier. MB3D latitude convention.
        float power = p.x;
        float r = max(length(z), 1e-12);
        float theta = atan(z.y, z.x) * power;
        float phi = asin(clamp(z.z / r, -1.0, 1.0)) * power;
        dr = power * pow(r, power - 1.0) * dr + seedDr;
        vec3 bulb = vec3(cos(phi) * cos(theta), cos(phi) * sin(theta), p.y * sin(phi));
        z = rotateBy(rot, bulb * pow(r, power) + seed);
    } else if (f == 8) {
        // Transform: scale, offset.
        if (jac) {
            J = rotateColumns(rot, J) * p.x;
        }
        z = rotateBy(rot, z) * p.x + p.yzw;
        dr *= abs(p.x);
    } else if (f == 9) {
        // Helispiral: twist per radius, per height, fixed.
        float rho = length(z.xy);
        float angle = p.z + p.x * rho + p.y * z.z;
        float sa = sin(angle);
        float ca = cos(angle);
        if (jac) {
            // The turn, plus the turn's own change along each column.
            mat3 turn = mat3(ca, sa, 0.0, -sa, ca, 0.0, 0.0, 0.0, 1.0);
            vec3 across = vec3(-sa * z.x - ca * z.y, ca * z.x - sa * z.y, 0.0);
            vec3 outward = rho > 1e-12 ? vec3(z.xy / rho, 0.0) : vec3(0.0);
            vec3 slope = outward * p.x + vec3(0.0, 0.0, p.y);
            J = rotateColumns(rot, turn * J + columnTimesRow(across, slope * J));
        }
        z = rotateBy(rot, vec3(ca * z.x - sa * z.y, sa * z.x + ca * z.y, z.z));
        dr *= 1.0 + rho * length(p.xy);
    } else if (f == 10) {
        // Gnarl: step, alpha, beta, scale. A zero scale counts as 1.
        float scale = p.w == 0.0 ? 1.0 : p.w;
        vec3 q = z;
        vec3 warped = vec3(
            gnarlAxis(q.x, q.z, p.x, p.y, p.z),
            gnarlAxis(q.y, q.x, p.x, p.y, p.z),
            gnarlAxis(q.z, q.y, p.x, p.y, p.z));
        if (jac) {
            vec3 slopes = vec3(gnarlSlope(q.z, p.x, p.y, p.z), gnarlSlope(q.x, p.x, p.y, p.z), gnarlSlope(q.y, p.x, p.y, p.z));
            mat3 shear = mat3(1.0, -slopes.y, 0.0, 0.0, 1.0, -slopes.z, -slopes.x, 0.0, 1.0);
            J = rotateColumns(rot, shear * J * scale);
        }
        z = rotateBy(rot, warped * scale);
        dr *= abs(scale) * (1.0 + abs(p.x) * (1.0 + abs(p.y) * (1.0 + abs(p.z))));
    } else if (f == 11) {
        // Bulbox P-2: scale, inner radius, inner scale, fold limit. Variant 2
        // keeps blending outside radius 1.
        vec3 folds = signs(z + p.w) - signs(z - p.w) - 1.0;
        vec3 folded = abs(z + p.w) - abs(z - p.w) - z;
        float r2 = max(dot(folded, folded), MIN_RADIUS2);
        float r = sqrt(r2);
        vec3 v = folded * p.x;
        float outer = abs(p.x);
        vec3 next;
        float stretch;
        mat3 dv = mat3(0.0);
        if (jac) {
            dv = timesEach(J, folds) * p.x;
        }
        if (mode == 0 && r >= 1.0) {
            next = v + seed;
            stretch = outer;
            if (jac) {
                J = dv + S;
            }
        } else {
            float rxy = max(sqrt(v.x * v.x + v.y * v.y), 1e-12);
            float q = p.z / (r2 * r2);
            float a = 1.0 - (v.z / rxy) * (v.z / rxy);
            vec3 t = vec3((v.x * v.x - v.y * v.y) * a * q, -(v.x * v.y) * a * q, -2.0 * v.z * rxy * q);
            float inner = 2.0 * abs(p.z) * p.x * p.x / (r2 * r);
            mat3 dt = mat3(0.0);
            if (jac) {
                dt = bulboxPowerJacobian(v, rxy, a, q) * dv;
            }
            if (r <= p.y) {
                next = t + seed;
                stretch = inner;
                if (jac) {
                    J = dt + S;
                }
            } else {
                float k = (r - p.y) / (1.0 - p.y);
                next = t * (1.0 - k) + v * k + seed;
                stretch = inner * abs(1.0 - k) + outer * abs(k);
                // k grows with the radius, which is |v| over the scale.
                if (jac) {
                    vec3 kSlope = folded / (r * p.x * (1.0 - p.y));
                    J = dt * (1.0 - k) + dv * k + columnTimesRow(v - t, kSlope * dv) + S;
                }
            }
        }
        if (jac) {
            J = rotateColumns(rot, J);
        }
        z = rotateBy(rot, next);
        dr = dr * stretch + seedDr;
    } else if (f == 12) {
        // Sphere inversion: radius, center. Variant 2 inverts the seed too,
        // which inverts the whole space the following slots see.
        vec3 center = p.yzw;
        vec3 d = z - center;
        float dd = dot(d, d);
        float k = p.x * p.x / max(dd, MIN_RADIUS2);
        dr *= k;
        if (jac) {
            if (dd > MIN_RADIUS2) {
                J = radialColumns(J, d, d, dd);
            }
            J = rotateColumns(rot, J * k);
        }
        z = rotateBy(rot, center + (z - center) * k);
        if (mode == 1) {
            vec3 ds = seed - center;
            float sdd = dot(ds, ds);
            float sk = p.x * p.x / max(sdd, MIN_RADIUS2);
            if (jacSeed) {
                if (sdd > MIN_RADIUS2) {
                    S = radialColumns(S, ds, ds, sdd);
                }
                S = rotateColumns(rot, S * sk);
            }
            seed = rotateBy(rot, center + ds * sk);
            seedDr *= sk;
        }
    } else if (f == 13) {
        // Polyfold Sym: order, angle shift (degrees), shift x, shift y. Each
        // sector turns back onto the first, mirrored when its index is odd.
        float x = z.x + p.z;
        float y = z.y + p.w;
        float order = abs(p.x) < 1e-9 ? 1.0 : p.x;
        float shift = radians(p.y);
        float n = roundEven((atan(y, x) + shift) * order / 6.283185307179586);
        float a = shift - n * 6.283185307179586 / order;
        float sa = sin(a);
        float ca = cos(a);
        float turnedY = x * sa + y * ca;
        bool even = (int(n) & 1) == 0;
        float mirror = even ? -1.0 : 1.0;
        if (jac) {
            mat3 turn = mat3(-ca, mirror * sa, 0.0, sa, mirror * ca, 0.0, 0.0, 0.0, 1.0);
            J = rotateColumns(rot, turn * J);
        }
        z = rotateBy(rot, vec3(y * sa - x * ca - p.z, mirror * turnedY - p.w, z.z));
    } else if (f == 14) {
        // Sine: offset 1, scale 1, scale 2, offset 2 on one component.
        // Variant 1 is y (MB3D _SinY), then x, then z.
        int axis = mode == 1 ? 0 : (mode >= 2 ? 2 : 1);
        vec3 w = z;
        w[axis] = sin((z[axis] - p.x) * p.y) * p.z + p.w;
        if (jac) {
            vec3 slope = vec3(1.0);
            slope[axis] = p.y * p.z * cos((z[axis] - p.x) * p.y);
            J = rotateColumns(rot, timesEach(J, slope));
        }
        z = rotateBy(rot, w);
        dr *= max(abs(p.y * p.z), 1.0);
    } else if (f == 15) {
        // Reciprocal (MB3D _reciprocalX3): limiter; variant picks the axis.
        int axis = clamp(mode, 0, 2);
        float limiter = max(abs(p.x), 1e-3);
        float c = z[axis];
        vec3 w = z;
        w[axis] = sign(c) * (1.0 / limiter - 1.0 / (abs(c) + limiter));
        if (jac) {
            vec3 slope = vec3(1.0);
            slope[axis] = 1.0 / ((abs(c) + limiter) * (abs(c) + limiter));
            J = rotateColumns(rot, timesEach(J, slope));
        }
        z = rotateBy(rot, w);
        dr *= max(1.0 / ((abs(c) + limiter) * (abs(c) + limiter)), 1.0);
    } else if (f == 16) {
        // Repeat: cell size, copies each way (0: endless), warp, warp phase.
        // The world tiled into cells, every other one mirrored unless
        // variant 2, after a warp that bends each cell differently. The seed
        // moves too.
        float size = max(abs(p.x), 1e-6);
        float stretch;
        float seedStretch;
        if (jac) {
            J = rotateColumns(rot, repeatColumns(J, z, size, p.y, mode != 1, p.z, p.w));
        }
        if (jacSeed) {
            S = rotateColumns(rot, repeatColumns(S, seed, size, p.y, mode != 1, p.z, p.w));
        }
        z = rotateBy(rot, repeatCells(quasiWarp(z, size, p.z, p.w, stretch), p.x, p.y, mode != 1));
        dr *= stretch;
        seed = rotateBy(rot, repeatCells(quasiWarp(seed, size, p.z, p.w, seedStretch), p.x, p.y, mode != 1));
        seedDr *= seedStretch;
    } else if (f == 17) {
        // Koch Cube (Luca G.N.): post-scale, XY stretch, Z fold, X add. No seed.
        float stretch = p.y == 0.0 ? 1.0 : p.y;
        if (jac) {
            J = timesEach(J, signs(z) * 3.0);
        }
        z = abs(z) * 3.0;
        if (swapIfLess(z.x, z.y) && jac) {
            J = exchangeRows(J, 0, 1, false);
        }
        if (swapIfLess(z.x, z.z) && jac) {
            J = exchangeRows(J, 0, 2, false);
        }
        if (swapIfLess(z.y, z.z) && jac) {
            J = exchangeRows(J, 1, 2, false);
        }
        z.x += p.w;
        z = rotateBy(rot, z);
        float near = 3.0 - stretch;
        float far = 3.0 + stretch;
        float foldZ = signs(p.z - z.z);
        z.z = p.z - abs(p.z - z.z);
        float x = z.x;
        float y = z.y;
        bool swapped = false;
        if (x - near < y) {
            z.x = x - near;
            z.y = y - near;
        } else if (x - far > y) {
            z.x = x - far;
        } else {
            z.x = y;
            z.y = x - far;
            swapped = true;
        }
        z.xy /= stretch;
        z *= p.x;
        if (jac) {
            J = timesEach(rotateColumns(rot, J), vec3(1.0, 1.0, foldZ));
            if (swapped) {
                J = exchangeRows(J, 0, 1, false);
            }
            J = timesEach(J, vec3(1.0 / stretch, 1.0 / stretch, 1.0) * p.x);
        }
        dr *= 3.0 * abs(p.x) * max(1.0 / abs(stretch), 1.0);
    } else if (f == 18) {
        // JCube: alpha, GScale, edge center (0, c, c), corner center (d, d, d).
        // No seed.
        float alpha = abs(p.x) < 1e-6 ? 1e-6 : p.x;
        if (jac) {
            J = timesEach(J, signs(z));
        }
        vec3 q = abs(z);
        // MB3D's partial sort: the smallest first, then max(min(x, y), z),
        // then max(x, y).
        bool xFirst = q.x < q.y;
        float a = xFirst ? q.x : q.y;
        float b = xFirst ? q.y : q.x;
        bool zSmallest = a >= q.z;
        vec3 sorted = zSmallest ? vec3(q.z, a, b) : vec3(a, q.z, b);
        if (jac) {
            // Rows follow the components: z's row moves first or second.
            mat3 rows = transpose(J);
            vec3 rx = rows[0];
            vec3 ry = rows[1];
            vec3 rz = rows[2];
            vec3 ra = xFirst ? rx : ry;
            vec3 rb = xFirst ? ry : rx;
            J = transpose(zSmallest ? mat3(rz, ra, rb) : mat3(ra, rz, rb));
        }
        float cornerScale = p.y - 1.0 + alpha;
        float edgeScale = cornerScale / alpha;
        bool edge = sorted.x <= 1.0 / edgeScale && sorted.y >= sorted.x + 1.0 - p.y / edgeScale;
        float scale = edge ? edgeScale : cornerScale;
        vec3 center = edge ? vec3(0.0, p.z, p.z) : vec3(p.w);
        z = rotateBy(rot, (sorted - center) * scale + center);
        if (jac) {
            J = rotateColumns(rot, J) * scale;
        }
        dr *= abs(scale);
    } else if (f == 19) {
        // Lin Combine XYZ: X, Y and Z multipliers, then the slot rotation.
        vec3 axes = p.xyz;
        z = rotateBy(rot, z * axes);
        if (jac) {
            J = rotateColumns(rot, timesEach(J, axes));
        }
        dr *= max(max(abs(p.x), abs(p.y)), abs(p.z));
    } else if (f == 20) {
        // Rotate 4D: XW, YW and ZW angles in half turns, then the slot rotation
        // for the YZ, XZ and XY planes. w starts at 0; only this turns z
        // through it.
        vec4 q = vec4(z, w);
        mat4 cols = mat4(vec4(J[0], Jw.x), vec4(J[1], Jw.y), vec4(J[2], Jw.z), vec4(0.0));
        for (int axis = 0; axis < 3; axis++) {
            float angle = radians(180.0 * (axis == 0 ? p.x : (axis == 1 ? p.y : p.z)));
            float sa = sin(angle);
            float ca = cos(angle);
            float a = q[axis];
            q[axis] = a * ca - q.w * sa;
            q.w = a * sa + q.w * ca;
            if (jac) {
                for (int k = 0; k < 3; k++) {
                    float ck = cols[k][axis];
                    float wk = cols[k].w;
                    cols[k][axis] = ck * ca - wk * sa;
                    cols[k].w = ck * sa + wk * ca;
                }
            }
        }
        z = rotateBy(rot, q.xyz);
        w = q.w;
        if (jac) {
            J = rotateColumns(rot, mat3(cols[0].xyz, cols[1].xyz, cols[2].xyz));
            Jw = vec3(cols[0].w, cols[1].w, cols[2].w);
        }
    } else if (f == 21) {
        // ABoxMod2: scale, min R, fold XY, fold Z; an inversion in a capped
        // cylinder of half size 0.5.
        vec3 folds = vec3(foldSigns(z, p.z).xy, foldSigns(z, p.w).z);
        vec3 folded = vec3(clamp(z.xy, -p.z, p.z) * 2.0 - z.xy, clamp(z.z, -p.w, p.w) * 2.0 - z.z);
        float cap = abs(folded.z) - 0.5;
        float rr = dot(folded.xy, folded.xy) + (cap > 0.0 ? cap * cap : 0.0);
        float min2 = max(p.y * p.y, MIN_RADIUS2);
        float m = p.x / clamp(rr, min2, 1.0);
        if (jac) {
            mat3 moved = timesEach(J, folds);
            if (rr > min2 && rr < 1.0) {
                vec3 grad = vec3(2.0 * folded.xy, cap > 0.0 ? 2.0 * cap * signs(folded.z) : 0.0);
                moved = moved - columnTimesRow(folded, (grad * moved) / rr);
            }
            J = rotateColumns(rot, moved * m + S);
        }
        z = rotateBy(rot, folded * m + seed);
        dr = dr * abs(m) + seedDr;
    } else if (f == 22) {
        // msltoe Sym4: XZ, XY and YZ sym-mul; conditional swaps and sign
        // flips, then the power-2 bulb and the seed. Power DE: J untouched.
        vec3 v = z;
        float r = length(v);
        if (abs(v.x) < abs(v.z) * p.x) {
            v.xz = v.zx;
        }
        if (abs(v.x) < abs(v.y) * p.y) {
            v.xy = v.yx;
        }
        if (abs(v.y) < abs(v.z) * p.z) {
            v.yz = v.zy;
        }
        if (v.x * v.z < 0.0) {
            v.z = -v.z;
        }
        if (v.x * v.y < 0.0) {
            v.y = -v.y;
        }
        z = rotateBy(rot, vec3(v.x * v.x - v.y * v.y - v.z * v.z, 2.0 * v.x * v.y, 2.0 * v.x * v.z) + seed);
        dr = 2.0 * r * dr + seedDr;
    }
}

// ── Stack (same rules as src/internal/fractal/stack.rs) ───────────────────

struct Sample {
    float d;
    float smoothIt;
    float trap;
    // Where the orbit came closest to the origin within its first
    // `surface_depth` iterations: surface texture coordinates that follow the
    // large structure, not the pixel-scale detail.
    vec3 trapPoint;
    float log2dr;
    float part;
    bool escaped;
};

// Fold bodies run by this pixel, for the work cap.
float g_work = 0.0;
// Iteration cap per chain; `max_iterations` is clamped to it.
const int MAX_ITERATIONS = 64;

float smoothIteration(float n, bool escaped, float prevR2, float r2) {
    if (!escaped || prevR2 <= 1.0 || bailout <= 1.0) {
        return n;
    }
    float d = log(0.5 * log(r2));
    float dPrev = log(0.5 * log(prevR2));
    if (abs(d - dPrev) < 1e-12) {
        return n;
    }
    return n + (log(log(bailout)) - d) / (d - dPrev);
}

// Visits per pass of slot K when it lies in [lo, end): 0 when empty.
#define VISITS(K, FORMULA, COUNT) ((FORMULA != 0 && K >= lo && K < end) ? max(COUNT, 0) : 0)

// Find the slot of visit t into the pass starting at slot `from`.
#define PICK(K, FORMULA, COUNT) \
    if (slot < 0 && FORMULA != 0 && K >= from && K < end) { \
        if (t < COUNT) { slot = K; } else { t -= COUNT; } \
    }

// Run slot K's formula if it is the picked slot.
#define APPLY(K, FORMULA, MODE, PA, PB, PC, PD, RX, RY, RZ) \
    if (FORMULA != 0 && slot == K) { \
        Rot rot; \
        rot.on = any(notEqual(vec3(RX, RY, RZ), vec3(0.0))); \
        if (rot.on) { \
            rot.r0 = flightTexel(ROTATION_TEXEL + 3 * K).xyz; \
            rot.r1 = flightTexel(ROTATION_TEXEL + 3 * K + 1).xyz; \
            rot.r2 = flightTexel(ROTATION_TEXEL + 3 * K + 2).xyz; \
        } \
        applyFormula(FORMULA, MODE, vec4(PA, PB, PC, PD), rot, z, dr, seed, seedDr, J, S, jac && i >= leadLen, jac && trackSeed, w4, Jw); \
    }

// Whether a formula may bend space unevenly, so its chain can need the
// Jacobian. The host decides from the parameters (distance kind 3).
#define BENDS(F) (F == 4 || (F >= 9 && F <= 12) || (F >= 14 && F <= 17) || (F >= 19 && F <= 21))
#define CYCLE_BENDS(K, F) (BENDS(F) && K >= repeat && K < end)

// Steps that act on the whole space, moving the seed with the point.
#define MOVES_SEED(F, M) (F == 16 || (F == 12 && M == 1))

// The leading run of space steps in the prefix, and whether any space step
// runs outside it, where the seed's Jacobian must be carried.
#define LEAD(K, F, M, C) \
    if (F != 0 && C > 0 && K >= first && K < end) { \
        if (MOVES_SEED(F, M) && leadingRun) { \
            leadLen += C; \
        } else { \
            leadingRun = false; \
            trackSeed = trackSeed || MOVES_SEED(F, M); \
        } \
        trackSeed = trackSeed || (MOVES_SEED(F, M) && K >= repeat); \
    }

// Run one chain from p. `lodFootprint` > 0 stops iterating once the
// structure left is smaller than that many world units.
//
// A part runs every active slot in [first, end) once (the prefix), then the
// slots from `repeat` on, forever (the cycle). All of that is constant, so
// the compiler folds the slot search to the few comparisons the stack needs.
Sample runChain(int part, vec3 p, float lodFootprint, vec4 kinds) {
    vec3 seed = julia_mode != 0u ? vec3(julia_x, julia_y, julia_z) : p;
    vec3 z = p;
    float dr = 1.0;
    float seedDr = julia_mode != 0u ? 0.0 : 1.0;
    float kind = part == 0 ? kinds.x : kinds.z;
    float planeZ = part == 0 ? kinds.y : kinds.w;
    mat3 J = mat3(1.0);
    // Rotate 4D's fourth coordinate and its gradient.
    float w4 = 0.0;
    vec3 Jw = vec3(0.0);
    mat3 S = mat3(1.0);
    if (julia_mode != 0u) {
        S = mat3(0.0);
    }
    float trap = dot(z, z);
    float surfaceTrap = trap;
    vec3 trapPoint = z;
    float prevR2 = trap;
    bool escaped = false;
    float n = 0.0;
    float bail2 = bailout * bailout;
    int cap = min(int(max_iterations), MAX_ITERATIONS);
    int split = clamp(SC_COMBINE_SPLIT - 1, 1, 5);
    int first = (SC_HYBRID_MODE == 1 && part == 1) ? split : 0;
    int end = (SC_HYBRID_MODE == 1 && part == 0) ? split : 6;
    int repeat = part == 0 ? clamp(SC_REPEAT_FROM - 1, first, end) : first;
    int lo = first;
    int prefixLen = VISITS(0, SC_SLOT1_FORMULA, SC_SLOT1_COUNT) + VISITS(1, SC_SLOT2_FORMULA, SC_SLOT2_COUNT)
        + VISITS(2, SC_SLOT3_FORMULA, SC_SLOT3_COUNT) + VISITS(3, SC_SLOT4_FORMULA, SC_SLOT4_COUNT)
        + VISITS(4, SC_SLOT5_FORMULA, SC_SLOT5_COUNT) + VISITS(5, SC_SLOT6_FORMULA, SC_SLOT6_COUNT);
    lo = repeat;
    int cycleLen = VISITS(0, SC_SLOT1_FORMULA, SC_SLOT1_COUNT) + VISITS(1, SC_SLOT2_FORMULA, SC_SLOT2_COUNT)
        + VISITS(2, SC_SLOT3_FORMULA, SC_SLOT3_COUNT) + VISITS(3, SC_SLOT4_FORMULA, SC_SLOT4_COUNT)
        + VISITS(4, SC_SLOT5_FORMULA, SC_SLOT5_COUNT) + VISITS(5, SC_SLOT6_FORMULA, SC_SLOT6_COUNT);
    if (cycleLen == 0) {
        repeat = first;
        cycleLen = prefixLen;
    }
    // Only a bend in the repeating part compounds; one that runs once keeps
    // its scalar bound.
    bool bends = CYCLE_BENDS(0, SC_SLOT1_FORMULA) || CYCLE_BENDS(1, SC_SLOT2_FORMULA)
        || CYCLE_BENDS(2, SC_SLOT3_FORMULA) || CYCLE_BENDS(3, SC_SLOT4_FORMULA)
        || CYCLE_BENDS(4, SC_SLOT5_FORMULA) || CYCLE_BENDS(5, SC_SLOT6_FORMULA);
    bool jac = bends && kind > 2.5;
    // Leading space steps move the point and the seed alike: after them J
    // restarts from the identity, and their stretch, dr so far, is a factor.
    // Only constants decide trackSeed, so S folds away when it is false; a
    // Julia seed's S is zero and stays zero either way.
    int leadLen = 0;
    bool leadingRun = true;
    bool trackSeed = false;
    LEAD(0, SC_SLOT1_FORMULA, SC_SLOT1_MODE, SC_SLOT1_COUNT)
    LEAD(1, SC_SLOT2_FORMULA, SC_SLOT2_MODE, SC_SLOT2_COUNT)
    LEAD(2, SC_SLOT3_FORMULA, SC_SLOT3_MODE, SC_SLOT3_COUNT)
    LEAD(3, SC_SLOT4_FORMULA, SC_SLOT4_MODE, SC_SLOT4_COUNT)
    LEAD(4, SC_SLOT5_FORMULA, SC_SLOT5_MODE, SC_SLOT5_COUNT)
    LEAD(5, SC_SLOT6_FORMULA, SC_SLOT6_MODE, SC_SLOT6_COUNT)
    if (julia_mode != 0u) {
        leadLen = 0;
    }
    float leadStretch = 1.0;
    for (int i = 0; i < MAX_ITERATIONS; i++) {
        float stretch = jac ? frobenius(J) * leadStretch : abs(dr);
        if (prefixLen == 0 || i >= cap || (lodFootprint > 0.0 && stretch * lodFootprint > 1.0)) {
            break;
        }
        int t = i;
        int from = first;
        if (i >= prefixLen) {
            t = (i - prefixLen) % cycleLen;
            from = repeat;
        }
        int slot = -1;
        PICK(0, SC_SLOT1_FORMULA, SC_SLOT1_COUNT)
        PICK(1, SC_SLOT2_FORMULA, SC_SLOT2_COUNT)
        PICK(2, SC_SLOT3_FORMULA, SC_SLOT3_COUNT)
        PICK(3, SC_SLOT4_FORMULA, SC_SLOT4_COUNT)
        PICK(4, SC_SLOT5_FORMULA, SC_SLOT5_COUNT)
        PICK(5, SC_SLOT6_FORMULA, SC_SLOT6_COUNT)
        prevR2 = dot(z, z);
        APPLY(0, SC_SLOT1_FORMULA, SC_SLOT1_MODE, slot1_a, slot1_b, slot1_c, slot1_d, slot1_rot_x, slot1_rot_y, slot1_rot_z)
        APPLY(1, SC_SLOT2_FORMULA, SC_SLOT2_MODE, slot2_a, slot2_b, slot2_c, slot2_d, slot2_rot_x, slot2_rot_y, slot2_rot_z)
        APPLY(2, SC_SLOT3_FORMULA, SC_SLOT3_MODE, slot3_a, slot3_b, slot3_c, slot3_d, slot3_rot_x, slot3_rot_y, slot3_rot_z)
        APPLY(3, SC_SLOT4_FORMULA, SC_SLOT4_MODE, slot4_a, slot4_b, slot4_c, slot4_d, slot4_rot_x, slot4_rot_y, slot4_rot_z)
        APPLY(4, SC_SLOT5_FORMULA, SC_SLOT5_MODE, slot5_a, slot5_b, slot5_c, slot5_d, slot5_rot_x, slot5_rot_y, slot5_rot_z)
        APPLY(5, SC_SLOT6_FORMULA, SC_SLOT6_MODE, slot6_a, slot6_b, slot6_c, slot6_d, slot6_rot_x, slot6_rot_y, slot6_rot_z)
        if (jac && i + 1 == leadLen) {
            leadStretch = abs(dr);
            J = mat3(1.0);
            if (trackSeed) {
                S = mat3(1.0);
            }
        }
        trap = min(trap, dot(z, z));
        if (n < surface_depth && dot(z, z) < surfaceTrap) {
            surfaceTrap = dot(z, z);
            trapPoint = z;
        }
        n += 1.0;
        g_work += 1.0;
        if (dot(z, z) > bail2) {
            escaped = true;
            break;
        }
    }
    float r = length(z);
    float adr = max(jac ? frobenius(J) * leadStretch : abs(dr), 1e-30);
    float d;
    if (kind < 0.5 || kind > 2.5) {
        // Linear, or Jacobian: adr is then the Jacobian's Frobenius norm.
        d = r / adr;
    } else if (kind > 1.5) {
        d = 0.5 * abs(z.z - planeZ) / adr;
    } else {
        d = 0.5 * log(max(r, 1e-30)) * r / adr;
    }
    Sample smp;
    smp.d = d;
    smp.smoothIt = smoothIteration(n, escaped, prevR2, r * r);
    smp.trap = trap;
    smp.trapPoint = trapPoint;
    smp.log2dr = log2(adr);
    smp.part = float(part);
    smp.escaped = escaped;
    return smp;
}

float combine(float d1, float d2, float width) {
    if (combine_op == 1) {
        return max(d1, d2);
    }
    if (combine_op == 2) {
        return max(-d1, d2);
    }
    if (combine_op == 3) {
        return min(d2 - max(width - d1, 0.0), d1 - max(width - d2, 0.0));
    }
    if (combine_op == 4 && width > 0.0) {
        float r1 = max(width - d1, 0.0);
        float r2 = max(width - d2, 0.0);
        return min(d2 - r1 * (width - r2) / width, d1 - r2 * (width - r1) / width);
    }
    return min(d1, d2);
}

// The stack's sample at p. `footprint` is one pixel in world units there;
// it scales the combine width and, through `lod`, the iteration cut.
Sample stackSample(vec3 p, float footprint, float lodPx) {
    vec4 kinds = flightTexel(KIND_TEXEL);
    float lodFootprint = lodPx > 0.0 ? lodPx * footprint : 0.0;
    Sample a = runChain(0, p, lodFootprint, kinds);
    if (SC_HYBRID_MODE != 1) {
        return a;
    }
    Sample b = runChain(1, p, lodFootprint, kinds);
    float d = combine(a.d, b.d, combine_width * footprint);
    Sample o = a;
    if (abs(b.d) < abs(a.d)) {
        o = b;
    }
    o.d = d;
    return o;
}

// ── Camera ───────────────────────────────────────────────────────────────

struct Camera {
    vec3 high;
    vec3 low;
    vec3 right;
    vec3 up;
    vec3 forward;
    float fov;
};

Camera camera() {
    Camera c;
    vec4 t0 = flightTexel(0);
    c.high = t0.xyz;
    c.fov = t0.w;
    c.low = flightTexel(1).xyz;
    c.right = flightTexel(2).xyz;
    c.up = flightTexel(3).xyz;
    c.forward = flightTexel(4).xyz;
    return c;
}

// ── G-buffer pass ─────────────────────────────────────────────────────────

const float FAR = 200.0;
const float MISS = 1e30;
// Fold bodies one pixel may run, counted where they run.
const float WORK_CAP = 24000.0;

// Point along a ray. The camera's high part is added last so small offsets
// keep their precision.
vec3 rayPoint(Camera c, vec3 dir, float t) {
    return c.high + (c.low + dir * t);
}

vec3 tetraNormal(Camera c, vec3 dir, float t, float footprint) {
    vec3 p = rayPoint(c, dir, t);
    float e = 0.5 * footprint;
    const vec2 k = vec2(1.0, -1.0);
    return normalize(
        k.xyy * stackSample(p + k.xyy * e, footprint, lod).d +
        k.yyx * stackSample(p + k.yyx * e, footprint, lod).d +
        k.yxy * stackSample(p + k.yxy * e, footprint, lod).d +
        k.xxx * stackSample(p + k.xxx * e, footprint, lod).d);
}

// Parity grid: the stack at a fixed lattice, for the host comparison test.
vec3 parityPoint(vec2 frag, vec2 size) {
    vec2 q = frag / size;
    return vec3(-3.0 + 6.0 * q.x, -3.0 + 6.0 * q.y, 0.37);
}

// The view ray through a full-resolution pixel center, top-left origin.
vec3 viewRay(Camera c, vec2 fragTopLeft, vec2 size, out float pixelAngle) {
    vec2 frag = vec2(fragTopLeft.x, size.y - fragTopLeft.y);
    // The image spans the whole frame at any size, so the aspect is the
    // output's: a render size can round to a slightly different one.
    vec2 sp = (2.0 * frag / size - 1.0) * vec2(outputAspect(), 1.0);
    float focal = 1.0 / tan(0.5 * c.fov);
    pixelAngle = 2.0 / (focal * size.y);
    return normalize(c.right * sp.x + c.up * sp.y + c.forward * focal);
}

// March one ray per 8x8 tile and keep how far the whole tile is provably in
// open space: from t to t + dt the tile's cone widens to (t + dt) * cone, and
// the empty sphere of radius d covers it while dt + (t + dt) * cone <= d.
// Uses the fine footprint, since a coarser level of detail is a different
// surface.
void tilesPass() {
    vec2 size = frameSize();
    Camera c = camera();
    float pixelAngle;
    vec2 center = (floor(gl_FragCoord.xy) + 0.5) * float(TILE);
    vec3 dir = viewRay(c, center, size, pixelAngle);
    // Half the tile diagonal as an angle, plus one pixel of jitter.
    float cone = (0.7072 * float(TILE) + 1.0) * pixelAngle;
    float t = 0.0;
    for (int i = 0; i < 64; i++) {
        float footprint = max(t, 1e-6) * pixelAngle;
        float d = stackSample(rayPoint(c, dir, t), footprint, lod).d;
        float dt = (d - t * cone) / (1.0 + cone);
        if (dt < footprint || t > FAR) {
            break;
        }
        t += dt;
    }
    out0 = vec4(0.9 * t, 0.0, 0.0, 1.0);
}

// Texture coordinates from the orbit, the way Machina maps its PBR textures:
// they follow the fractal's own structure instead of world space.
vec2 surfaceUv(vec3 trapPoint, float smoothIt) {
    if (surface_mapping == 1) {
        float r = max(length(trapPoint), 1e-6);
        return vec2(atan(trapPoint.y, trapPoint.x) / 6.283185307, acos(clamp(trapPoint.z / r, -1.0, 1.0)) / 3.14159265) * 4.0;
    }
    if (surface_mapping == 2) {
        return vec2(smoothIt * 0.25, length(trapPoint));
    }
    return trapPoint.xy;
}

void gbufferPass() {
    vec2 size = frameSize();
    if (debug_view == 6) {
        Sample s = stackSample(parityPoint(gl_FragCoord.xy, size), 0.0, 0.0);
        // Alpha stays 1: the composite would drop a transparent pixel.
        out0 = vec4(s.d, s.smoothIt, s.log2dr, 1.0);
        out1 = vec4(0.0);
        return;
    }
    Camera c = camera();
    float pixelAngle;
    vec3 dir = viewRay(c, jittered(gl_FragCoord.xy), size, pixelAngle);
    // Hits, detail and the normal are measured in output pixels, so geometry
    // keeps its thickness as the live render scale moves; TAA gathers the
    // sparser samples.
    pixelAngle *= size.y / outputSize().y;

    float t = 0.0;
    if (tile_prepass != 0u) {
        t = texelFetch(sampler2D(tiles, texSampler), ivec2(gl_FragCoord.xy) / TILE, 0).r;
    }
    float lastD = MISS;
    float lastStep = 0.0;
    float steps = 0.0;
    float exitCause = 2.0; // 0 hit, 1 soft hit, 2 miss, 3 budget
    float closestRatio = MISS;
    float closestT = 0.0;
    // Iteration fog: march steps counted, MB3D style. Steps shortened by
    // the relaxation count as a fraction.
    float fogCount = 0.0;
    int fogIteration = int(fog_on_iteration);
    Sample hit;
    hit.d = MISS;
    int maxSteps = int(max_steps);
    for (int i = 0; i < 512; i++) {
        if (i >= maxSteps || g_work > WORK_CAP) {
            exitCause = g_work > WORK_CAP ? 3.0 : 2.0;
            break;
        }
        float footprint = max(t, 1e-6) * pixelAngle;
        float stopDist = detail * footprint;
        Sample s = stackSample(rayPoint(c, dir, t), footprint, lod);
        float d = s.d;
        if (i > 0) {
            d = min(d, lastD + lastStep);
        }
        steps += 1.0;
        if (d < stopDist) {
            exitCause = 0.0;
            hit = s;
            break;
        }
        float ratio = d / stopDist;
        if (ratio < closestRatio) {
            closestRatio = ratio;
            closestT = t;
            hit = s;
        }
        float relax = (i > 0 && lastD > d) ? clamp(lastStep / (lastD - d), 0.5, 1.0) : 1.0;
        float safe = max(d * step_mult * relax, 0.1 * stopDist);
        if (fogIteration == 0 || int(s.smoothIt) == fogIteration) {
            fogCount += relax;
        }
        lastD = d;
        lastStep = safe;
        t += safe;
        if (t > FAR) {
            exitCause = 2.0;
            break;
        }
    }

    if (exitCause != 0.0 && closestRatio < 4.0) {
        // Out of steps in a crevice: use the closest approach.
        exitCause = exitCause == 3.0 ? 3.0 : 1.0;
        t = closestT;
    } else if (exitCause == 0.0) {
        // Bisect toward the stop shell between the last two positions.
        float lo = max(t - lastStep, 0.0);
        float hi = t;
        for (int k = 0; k < 6; k++) {
            float mid = 0.5 * (lo + hi);
            float footprint = max(mid, 1e-6) * pixelAngle;
            if (stackSample(rayPoint(c, dir, mid), footprint, lod).d < detail * footprint) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        t = hi;
    }

    bool surface = exitCause < 1.5;
    float footprint = max(t, 1e-6) * pixelAngle;
    vec3 n = surface ? tetraNormal(c, dir, t, footprint) : -dir;
    if (surface && dot(n, dir) > 0.0) {
        n = -n;
    }

    if (debug_view == 1) {
        out0 = vec4(surface ? t : MISS, 0.0, 0.0, steps);
        out1 = vec4(steps / float(maxSteps), g_work / WORK_CAP, 0.0, 0.0);
        return;
    }
    if (debug_view == 4) {
        out0 = vec4(surface ? t : MISS, 0.0, 0.0, steps);
        vec3 cause = exitCause == 0.0 ? vec3(0.0, 1.0, 0.0)
            : exitCause == 1.0 ? vec3(1.0, 1.0, 0.0)
            : exitCause == 2.0 ? vec3(0.0, 0.0, 1.0)
            : vec3(1.0, 0.0, 0.0);
        out1 = vec4(cause, 0.0);
        return;
    }
    out0 = vec4(surface ? t : MISS, hit.smoothIt, hit.trap, fogCount);
    out2 = vec4(surfaceUv(hit.trapPoint, hit.smoothIt) * surface_scale, 0.0, 0.0);
    float partSign = hit.part > 0.5 ? -1.0 : 1.0;
    out1 = vec4(n, partSign * max(hit.log2dr, 1e-3));
}

// ── Reprojection ─────────────────────────────────────────────────────────

bool reprojectVector(vec3 v, float fov, out vec2 prevUv, out float prevDist);

// Where world point `p` was on screen last frame, as a top-left uv, and how
// far it was from last frame's camera. False when it was behind the camera
// or off screen.
bool reproject(vec3 p, float fov, out vec2 prevUv, out float prevDist) {
    vec3 v = (p - flightTexel(5).xyz) - flightTexel(6).xyz;
    return reprojectVector(v, fov, prevUv, prevDist);
}

// Where a direction at infinity was on screen last frame: rotation only.
bool reprojectDirection(vec3 dir, float fov, out vec2 prevUv) {
    float unused;
    return reprojectVector(dir, fov, prevUv, unused);
}

// `v` from last frame's camera.
bool reprojectVector(vec3 v, float fov, out vec2 prevUv, out float prevDist) {
    float z = dot(v, flightTexel(9).xyz);
    if (z <= 0.0) {
        return false;
    }
    float focal = 1.0 / tan(0.5 * fov);
    vec2 sp = vec2(dot(v, flightTexel(7).xyz), dot(v, flightTexel(8).xyz)) * focal / z;
    vec2 at = 0.5 * (sp / vec2(outputAspect(), 1.0) + 1.0);
    prevUv = vec2(at.x, 1.0 - at.y);
    prevDist = length(v);
    return all(greaterThanEqual(prevUv, vec2(0.0))) && all(lessThanEqual(prevUv, vec2(1.0)));
}

// Weight of the newest frame when accumulating: all of it when history is
// missing or the formula is changing.
float newFrameWeight(float base) {
    if (HISTORYVALID == 0) {
        return 1.0;
    }
    return mix(base, 1.0, clamp(flightTexel(10).x, 0.0, 1.0));
}

// ── Shade pass: key-light shadow and occlusion, render size ──────────────

vec3 sunDirection() {
    return normalize(vec3(cos(sun_elev) * sin(sun_azim), sin(sun_elev), cos(sun_elev) * cos(sun_azim)));
}

vec3 fillDirection() {
    return normalize(vec3(cos(fill_elev) * sin(fill_azim), sin(fill_elev), cos(fill_elev) * cos(fill_azim)));
}

// Soft shadow toward `l`, MB3D's min(k h / t) penumbra with a far fade.
float softShadow(vec3 p, vec3 l, float footprint, float maxT) {
    float res = 1.0;
    float t = 2.0 * footprint;
    for (int i = 0; i < 48; i++) {
        if (t > maxT || g_work > WORK_CAP) {
            break;
        }
        float h = stackSample(p + l * t, footprint, 2.0 * lod).d;
        res = min(res, shadow_softness * h / t + pow(t / maxT, 8.0));
        if (res < 0.02) {
            return 0.0;
        }
        t += max(h, footprint);
    }
    return clamp(res, 0.0, 1.0);
}

// Three cone rays at 30 degrees from the normal, rotated per frame, each
// stepping outward geometrically (MB3D DEAO, lowest quality, accumulated
// over frames instead).
float occlusion(vec3 p, vec3 n, float footprint, float spin) {
    vec3 a = normalize(abs(n.x) < 0.9 ? cross(n, vec3(1.0, 0.0, 0.0)) : cross(n, vec3(0.0, 1.0, 0.0)));
    vec3 b = cross(n, a);
    float total = 0.0;
    for (int r = 0; r < 3; r++) {
        float az = spin + 2.0943951 * float(r);
        vec3 dir = normalize(n * 0.866 + (a * cos(az) + b * sin(az)) * 0.5);
        float occ = 1.0;
        float t = 2.0 * footprint;
        for (int s = 0; s < 6; s++) {
            float d = stackSample(p + dir * t, footprint, 2.0 * lod).d;
            occ = min(occ, d / t);
            t *= 2.5;
        }
        total += clamp(occ, 0.0, 1.0);
    }
    return total / 3.0;
}

// Last frame's shadow and occlusion where a point was, from the four
// nearest shade texels: those within 5% of `prevDist` weighted bilinearly.
// A single bilinear tap would blend depths across pixel-scale relief and
// match nothing. False when no tap matches; `history` is then the
// nearest-depth tap's.
bool shadeHistory(vec2 prevUv, float prevDist, out vec2 history) {
    vec2 at = prevUv * previousFrameSize() - 0.5;
    ivec2 base = ivec2(floor(at));
    vec2 f = fract(at);
    ivec2 limit = ivec2(ceil(previousFrameSize())) - 1;
    vec2 sum = vec2(0.0);
    float weight = 0.0;
    float nearest = 1e30;
    history = vec2(1.0);
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec4 h = texelFetch(sampler2D(shade, texSampler), clamp(base + ivec2(i, j), ivec2(0), limit), 0);
            float gap = abs(h.z - prevDist);
            if (gap < nearest) {
                nearest = gap;
                history = h.xy;
            }
            if (gap < 0.05 * prevDist) {
                float bilinear = (i == 0 ? 1.0 - f.x : f.x) * (j == 0 ? 1.0 - f.y : f.y) + 1e-5;
                sum += h.xy * bilinear;
                weight += bilinear;
            }
        }
    }
    if (weight > 0.0) {
        history = sum / weight;
        return true;
    }
    return false;
}

void shadePass() {
    vec2 size = frameSize();
    ivec2 full = min(ivec2(gl_FragCoord.xy), ivec2(size) - 1);
    vec4 a = texelFetch(sampler2D(gA, texSampler), full, 0);
    if (a.x >= MISS * 0.5) {
        out0 = vec4(1.0, 1.0, MISS, 0.0);
        return;
    }
    Camera c = camera();
    float pixelAngle;
    vec3 dir = viewRay(c, jittered(vec2(full) + 0.5), size, pixelAngle);
    // In output pixels, as in the G-buffer, so shading does not change with
    // the render scale.
    pixelAngle *= size.y / outputSize().y;
    // Each frame computes one 8x8 block of every 2x2 blocks, in turn; the
    // others keep their reprojected history. Held still, the buffer is full
    // resolution after 4 frames at the cost of a quarter of its pixels.
    // Blocks, not single pixels: the GPU runs neighboring pixels together,
    // and a group with any pixel due would run the full cost.
    float w = newFrameWeight(0.15);
    vec2 prevUv;
    float prevDist;
    bool onScreen = w < 1.0 && reproject(rayPoint(c, dir, a.x), c.fov, prevUv, prevDist);
    vec2 history = vec2(0.0);
    bool haveHistory = onScreen && shadeHistory(prevUv, prevDist, history);
    ivec2 block = full >> 3;
    bool due = (block.x & 1) + 2 * (block.y & 1) == JITTERINDEX % 4;
    // The whole block takes the same branch, so a block that is not due
    // never pays for shading. A pixel without matching history keeps the
    // nearest surface's until its block is due.
    if (onScreen && !due) {
        out0 = vec4(history, a.x, 0.0);
        return;
    }

    vec3 n = texelFetch(sampler2D(gB, texSampler), full, 0).xyz;
    float footprint = max(a.x, 1e-6) * pixelAngle;
    vec3 p = rayPoint(c, dir, a.x) + n * (2.0 * footprint);

    float spin = float(JITTERINDEX) * 2.3999632;
    vec3 sun = sunDirection();
    float maxT = shadow_length * max(a.x, sceneScale());
    float shadow = dot(n, sun) > 0.0 ? softShadow(p, sun, footprint, maxT) : 0.0;
    float ao = occlusion(p, n, footprint, spin);
    vec3 current = vec3(shadow, ao, a.x);
    if (haveHistory) {
        // Updated every fourth frame: the weight four frames of 0.15 add up to.
        float every4 = 1.0 - pow(1.0 - w, 4.0);
        current.xy = mix(history, current.xy, every4);
    }
    out0 = vec4(current, 1.0);
}

// ── Light pass: materials and lights, full resolution ────────────────────

// Integral of the four-stop palette from 0 to `u`, in stops (four per
// cycle).
vec3 paletteIntegral(float u) {
    vec3 stops[4] = vec3[4](color1.rgb, color2.rgb, color3.rgb, color4.rgb);
    vec3 cycle = vec3(0.0);
    for (int k = 0; k < 4; k++) {
        cycle += 0.5 * (stops[k] + stops[(k + 1) % 4]);
    }
    float whole = floor(u / 4.0);
    float rest = u - whole * 4.0;
    vec3 sum = cycle * whole;
    for (int k = 0; k < 4; k++) {
        float f = clamp(rest - float(k), 0.0, 1.0);
        sum += stops[k] * f + (stops[(k + 1) % 4] - stops[k]) * 0.5 * f * f;
    }
    return sum;
}

// Four-stop cyclic palette, box filtered over `cycles`, the palette cycles
// one pixel spans: stripe edges are antialiased, and rings finer than a pixel
// show as their average instead of moire.
vec3 palette(float x, float cycles) {
    float u = (x * palette_scale + palette_offset) * 4.0;
    float width = max(cycles * 4.0, 1e-3);
    // Measured from the start of u's cycle, so the integral keeps precision.
    u -= floor(u / 4.0) * 4.0;
    return (paletteIntegral(u + 0.5 * width) - paletteIntegral(u - 0.5 * width)) / width;
}

// The palette index of a G-buffer sample.
float colorIndex(vec4 a, vec4 b) {
    if (color_source == 1) {
        return sqrt(a.z);
    } else if (color_source == 2) {
        return abs(b.w) * 0.05;
    } else if (color_source == 3) {
        return b.y * 0.5 + 0.5;
    } else if (color_source == 4) {
        return b.w < 0.0 ? 0.5 : 0.0;
    }
    return a.y * 0.1;
}

// Palette cycles per pixel at `texel`: the largest index step to the right
// and below neighbors on the same surface.
float paletteCycles(ivec2 texel, float index, float depth) {
    ivec2 limit = ivec2(ceil(frameSize())) - 1;
    float jump = 0.0;
    for (int k = 0; k < 2; k++) {
        ivec2 at = min(texel + (k == 0 ? ivec2(1, 0) : ivec2(0, 1)), limit);
        vec4 na = texelFetch(sampler2D(gA, texSampler), at, 0);
        if (na.x < MISS * 0.5 && abs(na.x - depth) < 0.05 * depth) {
            vec4 nb = texelFetch(sampler2D(gB, texSampler), at, 0);
            jump = max(jump, abs(colorIndex(na, nb) - index));
        }
    }
    return jump * palette_scale;
}

// Shadow and occlusion at a render position: the four nearest samples,
// weighted by how close their depth is. At a render pixel's center that is
// the pixel's own sample.
vec2 upsampleShade(vec2 fragTopLeft, float t) {
    vec2 at = fragTopLeft - 0.5;
    ivec2 base = ivec2(floor(at));
    vec2 f = fract(at);
    ivec2 limit = ivec2(ceil(frameSize())) - 1;
    vec2 sum = vec2(0.0);
    float weight = 0.0;
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec4 s = texelFetch(sampler2D(shade, texSampler), clamp(base + ivec2(i, j), ivec2(0), limit), 0);
            float bilinear = (i == 0 ? 1.0 - f.x : f.x) * (j == 0 ? 1.0 - f.y : f.y);
            float w = bilinear * exp(-abs(s.z - t) / (0.02 * t + 1e-6)) + 1e-5;
            sum += s.xy * w;
            weight += w;
        }
    }
    return sum / weight;
}

// Light shaft color at this pixel, from the quarter-resolution volume.
vec3 shaftLight() {
    if (shafts <= 0.0) {
        return vec3(0.0);
    }
    vec2 volSize = vec2(textureSize(sampler2D(vol, texSampler), 0));
    float amount = texture(sampler2D(vol, texSampler), liveUv(gl_FragCoord.xy, frameSize(), 4.0, volSize)).x;
    return sun_color.rgb * shafts * amount * 0.5;
}

float ggx(float nh, float rough) {
    float a2 = rough * rough * rough * rough;
    float d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * d * d);
}

vec3 sky(vec3 dir) {
    return mix(amb_bottom.rgb, amb_top.rgb, 0.5 + 0.5 * dir.y) * sky_brightness;
}

// ── Surface textures ─────────────────────────────────────────────────────

const float SURFACE_TILE = 512.0;
// Each material's mean linear color, the texture seen from too far to resolve.
const vec3 SURFACE_MEAN[6] = vec3[6](
    vec3(0.0800, 0.0918, 0.1055), vec3(0.1868, 0.1624, 0.0749), vec3(0.3301, 0.2814, 0.2561),
    vec3(0.1994, 0.1389, 0.0858), vec3(0.3241, 0.2990, 0.1062), vec3(0.0771, 0.1088, 0.1104));

// Texture the surface: color, a roughness from the height, and a bump by
// Mikkelsen's surface gradient, which needs no tangents. Where a pixel
// covers more texels than it can show (the atlas has no mipmaps), the
// texture fades to its mean color instead of shimmering.
// One material's color and height at uv, repeating.
vec4 surfaceSample(int tile, vec2 uv) {
    // Inset by a texel so bilinear taps never reach the neighboring tile.
    vec2 local = fract(uv) * (1.0 - 2.0 / SURFACE_TILE) + 1.0 / SURFACE_TILE;
    vec2 atlas = (vec2(float(tile % 3), float(tile / 3)) + local) / vec2(3.0, 2.0);
    return texture(sampler2D(surfaces, texSampler), atlas);
}


void applySurface(ivec2 texel, vec3 position, inout vec3 albedo, inout float rough, inout vec3 n) {
    int tile = clamp(surface - 1, 0, 5);
    vec4 s;
    float resolve;
    if (surface_mapping == 3) {
        // World: projected along each axis and blended by the normal
        // (triplanar). A tile is sized so its texels meet output pixels one
        // to one at the scene scale's distance: the texture shows at full
        // resolution wherever the camera is and at any output size. The
        // scene scale holds while walking, so the size holds too. The orbit
        // mappings jump between neighboring pixels at fold edges and fade
        // to the mean color almost everywhere.
        float outputPixel = 2.0 / (outputSize().y * (1.0 / tan(0.5 * camera().fov)));
        float tileWorld = SURFACE_TILE * sceneScale() * outputPixel / surface_scale;
        vec3 p = position / tileWorld;
        vec3 w = pow(abs(n), vec3(4.0));
        w /= max(w.x + w.y + w.z, 1e-6);
        // Where tiles fall below a pixel, take the texture at a coarser
        // octave instead of fading it: tiles grow with distance, blended
        // between neighboring octaves so no step shows, as mipmaps would.
        vec2 footprint = max(max(fwidth(p.yz), fwidth(p.zx)), fwidth(p.xy)) * SURFACE_TILE;
        float octave = max(log2(max(max(footprint.x, footprint.y), 1e-6)), 0.0);
        float k = floor(octave);
        float blend = octave - k;
        vec3 p0 = p * exp2(-k);
        vec3 p1 = p0 * 0.5;
        vec4 s0 = surfaceSample(tile, p0.yz) * w.x + surfaceSample(tile, p0.zx) * w.y + surfaceSample(tile, p0.xy) * w.z;
        vec4 s1 = surfaceSample(tile, p1.yz) * w.x + surfaceSample(tile, p1.zx) * w.y + surfaceSample(tile, p1.xy) * w.z;
        s = mix(s0, s1, blend);
        resolve = 1.0;
    } else {
        vec2 uv = texelFetch(sampler2D(gC, texSampler), texel, 0).xy;
        vec2 footprint = fwidth(uv) * SURFACE_TILE;
        resolve = 1.0 - smoothstep(0.75, 3.0, max(footprint.x, footprint.y));
        s = surfaceSample(tile, uv);
    }
    // A detail texture: the palette's color times the material's variation
    // around its own mean, so the surface gains grain and relief and keeps
    // the look's colors and brightness. Several materials are near black on
    // average and would otherwise darken the scene.
    vec3 variation = clamp(mix(vec3(1.0), s.rgb / max(SURFACE_MEAN[tile], vec3(0.02)), resolve), 0.0, 3.0);
    albedo = mix(albedo, albedo * variation, surface_amount);
    float height = mix(0.5, s.a, resolve);
    rough = mix(rough, 0.95 - 0.5 * height, surface_amount);

    vec3 dpdx = dFdx(position);
    vec3 dpdy = dFdy(position);
    float dhdx = dFdx(height);
    float dhdy = dFdy(height);
    vec3 r1 = cross(dpdy, n);
    vec3 r2 = cross(n, dpdx);
    float det = dot(dpdx, r1);
    vec3 gradient = sign(det) * (dhdx * r1 + dhdy * r2);
    vec3 bumped = abs(det) * n - surface_bump * surface_amount * gradient;
    if (dot(bumped, bumped) > 1e-20) {
        n = normalize(bumped);
    }
}

void lightPass() {
    vec2 size = frameSize();
    ivec2 texel = ivec2(gl_FragCoord.xy);
    vec4 a = texelFetch(sampler2D(gA, texSampler), texel, 0);
    vec4 b = texelFetch(sampler2D(gB, texSampler), texel, 0);
    Camera c = camera();
    float pixelAngle;
    vec3 dir = viewRay(c, jittered(gl_FragCoord.xy), size, pixelAngle);
    float scale = sceneScale();
    // Iteration fog glows in front of whatever the ray reached.
    vec3 glow = dyn_fog_color.rgb * dyn_fog * a.w * 0.02;

    if (a.x >= MISS * 0.5) {
        vec3 background = mix(sky(dir), fog_color.rgb, clamp(fog_density, 0.0, 1.0));
        out0 = vec4(background + glow + shaftLight(), 1.0);
        return;
    }

    vec3 n = b.xyz;
    float index = colorIndex(a, b);
    vec3 albedo = palette(index, paletteCycles(texel, index, a.x));
    float rough = roughness;
    if (surface > 0) {
        applySurface(texel, rayPoint(c, dir, a.x), albedo, rough, n);
    }
    vec2 sh = upsampleShade(gl_FragCoord.xy, a.x);
    float shadow = sh.x;
    float ao = mix(1.0, sh.y, ao_strength);

    vec3 v = -dir;
    vec3 f0 = mix(vec3(0.04), albedo, metallic);
    vec3 diffuseColor = albedo * (1.0 - metallic);
    vec3 color = vec3(0.0);

    // Key light, shadowed.
    vec3 l = sunDirection();
    float nl = max(dot(n, l), 0.0);
    vec3 h = normalize(l + v);
    vec3 fresnel = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    vec3 spec = fresnel * ggx(max(dot(n, h), 0.0), rough) * specular * 0.25;
    color += (diffuseColor + spec) * sun_color.rgb * sun_intensity * nl * shadow;

    // Fill light, unshadowed.
    vec3 lf = fillDirection();
    color += diffuseColor * fill_color.rgb * fill_intensity * max(dot(n, lf), 0.0) * ao;

    // Headlight at the camera, falling off with distance in units of the
    // camera's distance to the surface.
    float falloff = 1.0 / (1.0 + (a.x / scale) * (a.x / scale));
    color += diffuseColor * headlight_color.rgb * headlight_intensity * max(dot(n, v), 0.0) * falloff;

    // Sky and ground ambient, and bounce light from occluded crevices.
    color += diffuseColor * sky(n) * ao;
    color += albedo * albedo * sun_color.rgb * bounce * (1.0 - sh.y) * 0.5;

    // Glow band on the palette index.
    if (emit_width > 0.0) {
        float band = abs(fract(index * palette_scale + palette_offset - emit_band + 0.5) - 0.5);
        color += albedo * emit_gain * smoothstep(emit_width, 0.0, band);
    }

    // Depth fog in units of the distance to the surface.
    float fog = 1.0 - exp(-fog_density * a.x / (4.0 * scale));
    color = mix(color, fog_color.rgb, fog);
    out0 = vec4(color + glow + shaftLight(), 1.0);
}

// ── Volume pass: light shafts, quarter resolution ────────────────────────

float henyeyGreenstein(float cosTheta, float g) {
    float g2 = g * g;
    return (1.0 - g2) / (4.0 * 3.14159265 * pow(1.0 + g2 - 2.0 * g * cosTheta, 1.5));
}

// Whether the key light reaches p: a short, coarse shadow march.
float sunVisible(vec3 p, vec3 l, float footprint, float maxT) {
    float t = footprint;
    for (int i = 0; i < 12; i++) {
        float h = stackSample(p + l * t, footprint, 4.0 * lod).d;
        if (h < footprint) {
            return 0.0;
        }
        t += h;
        if (t > maxT) {
            break;
        }
    }
    return 1.0;
}

// In-scattered key light along the view ray up to the surface: jittered
// samples, each asking whether the sun reaches it, accumulated over frames.
void volumePass() {
    if (shafts <= 0.0) {
        out0 = vec4(0.0);
        return;
    }
    vec2 size = frameSize();
    ivec2 full = min(ivec2(gl_FragCoord.xy) * 4 + 2, ivec2(size) - 1);
    float surfaceT = texelFetch(sampler2D(gA, texSampler), full, 0).x;
    Camera c = camera();
    float pixelAngle;
    vec3 dir = viewRay(c, vec2(full) + 0.5, size, pixelAngle);
    float scale = sceneScale();
    float endT = min(surfaceT, 16.0 * scale);
    vec3 l = sunDirection();
    const int SAMPLES = 12;
    float offset = fract(float(JITTERINDEX) * 0.618034 + dot(gl_FragCoord.xy, vec2(0.0671, 0.0583)));
    float lit = 0.0;
    for (int i = 0; i < SAMPLES; i++) {
        float t = endT * (float(i) + offset) / float(SAMPLES);
        float footprint = max(t, 1e-6) * pixelAngle * 4.0;
        lit += sunVisible(rayPoint(c, dir, t), l, footprint, shadow_length * scale);
    }
    // Lit fraction of the path, times how much of the path counts (it
    // saturates over a few distances to the surface), times the phase
    // function normalized so isotropic scattering is 1.
    float path = 1.0 - exp(-endT / (8.0 * scale));
    float phase = 4.0 * 3.14159265 * henyeyGreenstein(dot(dir, l), shaft_anisotropy);
    float amount = lit / float(SAMPLES) * path * phase;
    float w = newFrameWeight(0.1);
    vec2 prevUv;
    float prevDist;
    if (w < 1.0 && reproject(rayPoint(c, dir, endT), c.fov, prevUv, prevDist)) {
        vec2 last = previousFrameSize();
        vec2 volSize = vec2(textureSize(sampler2D(vol, texSampler), 0));
        vec4 history = texture(sampler2D(vol, texSampler), liveUv(prevUv * last, last, 4.0, volSize));
        if (abs(history.y - prevDist) < 0.1 * prevDist) {
            amount = mix(history.x, amount, w);
        }
    }
    out0 = vec4(amount, endT, 0.0, 1.0);
}

// ── Depth of field, half resolution ──────────────────────────────────────

// The image at an output texel before sharpening: TAA's, or without it
// the lit image upsampled from the render size.
vec3 unsharpened(ivec2 texel) {
    return temporal != 0u
        ? texelFetch(sampler2D(taa, texSampler), texel, 0).rgb
        : hdrAtOutput((vec2(texel) + 0.5) / outputSize());
}

// This frame's resolved color at an output texel, sharpened.
vec3 resolved(ivec2 texel) {
    return texelFetch(sampler2D(sharp, texSampler), texel, 0).rgb;
}

// ── Sharpen pass: RCAS ───────────────────────────────────────────────────
//
// Robust contrast-adaptive sharpening from AMD FidelityFX Super Resolution 1
// (FsrRcasF, MIT; see THIRD_PARTY_NOTICES.md). It sharpens by the most it can
// without leaving the range of the 3x3 cross, and less where the cross looks
// like noise. RCAS assumes values in [0, 1], so it runs on tonemapped color.

const float RCAS_LIMIT = 0.25 - 1.0 / 16.0;

vec3 rcasIn(vec3 c) {
    return c / (1.0 + max(max(c.r, c.g), c.b));
}

vec3 rcasOut(vec3 c) {
    return c / max(1.0 - max(max(c.r, c.g), c.b), 1.0 / 32768.0);
}

void sharpPass() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    vec3 eIn = unsharpened(texel);
    if (sharpness <= 0.0) {
        out0 = vec4(eIn, 1.0);
        return;
    }
    ivec2 limit = ivec2(outputSize()) - 1;
    //    b
    //  d e f
    //    h
    vec3 b = rcasIn(unsharpened(clamp(texel + ivec2(0, -1), ivec2(0), limit)));
    vec3 d = rcasIn(unsharpened(clamp(texel + ivec2(-1, 0), ivec2(0), limit)));
    vec3 e = rcasIn(eIn);
    vec3 f = rcasIn(unsharpened(clamp(texel + ivec2(1, 0), ivec2(0), limit)));
    vec3 h = rcasIn(unsharpened(clamp(texel + ivec2(0, 1), ivec2(0), limit)));
    // Luma times 2.
    float bL = b.b * 0.5 + (b.r * 0.5 + b.g);
    float dL = d.b * 0.5 + (d.r * 0.5 + d.g);
    float eL = e.b * 0.5 + (e.r * 0.5 + e.g);
    float fL = f.b * 0.5 + (f.r * 0.5 + f.g);
    float hL = h.b * 0.5 + (h.r * 0.5 + h.g);
    // Noise detection.
    float nz = 0.25 * (bL + dL + fL + hL) - eL;
    float lumaRange = max(max(max(bL, dL), max(eL, fL)), hL) - min(min(min(bL, dL), min(eL, fL)), hL);
    nz = clamp(abs(nz) / max(lumaRange, 1e-6), 0.0, 1.0);
    nz = -0.5 * nz + 1.0;
    // Min and max of the ring, and the lobe that keeps the result inside it.
    vec3 mn4 = min(min(b, d), min(f, h));
    vec3 mx4 = max(max(b, d), max(f, h));
    vec3 hitMin = min(mn4, e) / (4.0 * mx4);
    vec3 hitMax = (1.0 - max(mx4, e)) / (4.0 * mn4 - 4.0);
    vec3 lobe3 = max(-hitMin, hitMax);
    float stops = 2.0 * (1.0 - sharpness);
    float lobe = max(-RCAS_LIMIT, min(max(max(lobe3.r, lobe3.g), lobe3.b), 0.0)) * exp2(-stops);
    lobe *= nz;
    vec3 c = (lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0);
    out0 = vec4(rcasOut(c), 1.0);
}

// Circle of confusion in full-resolution pixels at distance t.
float circleOfConfusion(float t, float frameHeight) {
    float maxCoc = aperture * 0.02 * frameHeight;
    if (dof_mode == 2) {
        // Ramp: near stays sharp, blur grows with distance.
        return maxCoc * clamp(t / (8.0 * sceneScale()), 0.0, 1.0);
    }
    float focus = autofocus != 0u ? max(flightTexel(1).w, 1e-6) : focus_distance * sceneScale();
    return maxCoc * clamp(abs(t - focus) / max(t, 1e-6), 0.0, 1.0);
}

void dofPass() {
    vec2 size = outputSize();
    ivec2 full = ivec2(gl_FragCoord.xy) * 2;
    float t = min(texelFetch(sampler2D(gA, texSampler), toRender(full), 0).x, 1e6);
    float coc = circleOfConfusion(t, size.y);
    vec3 sum = resolved(full);
    float weight = 1.0;
    ivec2 limit = ivec2(size) - 1;
    const int TAPS = 24;
    for (int i = 0; i < TAPS; i++) {
        // Golden-angle spiral over the disc.
        float r = sqrt((float(i) + 0.5) / float(TAPS));
        float a = float(i) * 2.3999632;
        vec2 offset = vec2(cos(a), sin(a)) * r * coc;
        ivec2 at = clamp(full + ivec2(offset), ivec2(0), limit);
        float tapT = min(texelFetch(sampler2D(gA, texSampler), toRender(at), 0).x, 1e6);
        float tapCoc = circleOfConfusion(tapT, size.y);
        // A tap counts where its own blur reaches this pixel, and never
        // when it is nearer than a sharp pixel (no halos over the background).
        float reach = clamp(tapCoc - length(offset) + 1.0, 0.0, 1.0);
        float behind = tapT >= t * 0.98 ? 1.0 : reach;
        float w = reach * behind;
        sum += resolved(at) * w;
        weight += w;
    }
    out0 = vec4(sum / weight, coc);
}

// ── Bloom, three levels ──────────────────────────────────────────────────

const int SOURCE_IMAGE = 0;
const int SOURCE_BLOOM1 = 1;
const int SOURCE_BLOOM2 = 2;

// Sample a bloom source at uv: the image the composite blurs (depth of
// field when on, else the resolved color), or a coarser bloom level.
vec3 bloomSource(int source, vec2 uv) {
    if (source == SOURCE_BLOOM1) {
        return texture(sampler2D(bloom1, texSampler), uv).rgb;
    }
    if (source == SOURCE_BLOOM2) {
        return texture(sampler2D(bloom2, texSampler), uv).rgb;
    }
    if (dof_mode != 0) {
        return texture(sampler2D(dof, texSampler), uv).rgb;
    }
    return texture(sampler2D(sharp, texSampler), uv).rgb;
}

// Four bilinear taps at the corners of this pixel's footprint in a source
// twice (or more) its resolution.
vec3 downsample(int source) {
    vec2 uv = gl_FragCoord.xy / RENDERSIZE;
    vec2 d = 0.5 / RENDERSIZE;
    return 0.25 * (bloomSource(source, uv + vec2(-d.x, -d.y)) + bloomSource(source, uv + vec2(d.x, -d.y))
        + bloomSource(source, uv + vec2(-d.x, d.y)) + bloomSource(source, uv + vec2(d.x, d.y)));
}

void bloomPass(int level) {
    if (bloom <= 0.0) {
        out0 = vec4(0.0);
        return;
    }
    vec3 c;
    if (level == 1) {
        c = downsample(SOURCE_IMAGE);
        float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
        c *= max(luma - bloom_threshold, 0.0) / max(luma, 1e-4);
    } else if (level == 2) {
        c = downsample(SOURCE_BLOOM1);
    } else {
        c = downsample(SOURCE_BLOOM2);
    }
    out0 = vec4(c, 1.0);
}

// ── TAA pass ─────────────────────────────────────────────────────────────

// Distance stored for sky pixels; f16 cannot hold MISS.
const float TAA_SKY = 60000.0;

vec3 toYCoCg(vec3 c) {
    return vec3(0.25 * c.r + 0.5 * c.g + 0.25 * c.b, 0.5 * c.r - 0.5 * c.b, -0.25 * c.r + 0.5 * c.g - 0.25 * c.b);
}

vec3 fromYCoCg(vec3 c) {
    return vec3(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

// Whether the center of a 3x3 of lumas (index 4) is a thin feature: a ridge
// brighter or darker than every neighbor that differs from it by more than
// 5%, with no 2x2 corner of similar neighbors that would make it part of a
// flat area. FSR 2's ComputeThinFeatureConfidence (MIT; see
// THIRD_PARTY_NOTICES.md).
bool thinFeature(float l[9]) {
    float nucleus = l[4];
    float dissimilarMin = 1e30;
    float dissimilarMax = 0.0;
    int similar = 1 << 4;
    for (int k = 0; k < 9; k++) {
        if (k == 4) {
            continue;
        }
        if (max(l[k], nucleus) / max(min(l[k], nucleus), 1e-6) < 1.05) {
            similar |= 1 << k;
        } else {
            dissimilarMin = min(dissimilarMin, l[k]);
            dissimilarMax = max(dissimilarMax, l[k]);
        }
    }
    if (!(nucleus > dissimilarMax || nucleus < dissimilarMin)) {
        return false;
    }
    const int CORNERS[4] = int[4](0x1B, 0x36, 0xD8, 0x1B0);
    for (int c = 0; c < 4; c++) {
        if ((similar & CORNERS[c]) == CORNERS[c]) {
            return false;
        }
    }
    return true;
}

// Last frame's color at uv, Catmull-Rom filtered in 5 bilinear taps
// (Jimenez), so reprojection under slow motion does not blur it.
vec3 catmullRomTaa(vec2 uv, vec2 size) {
    vec2 at = uv * size;
    vec2 p1 = floor(at - 0.5) + 0.5;
    vec2 f = at - p1;
    vec2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    vec2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    vec2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    vec2 w3 = f * f * (-0.5 + 0.5 * f);
    vec2 w12 = w1 + w2;
    vec2 p0 = (p1 - 1.0) / size;
    vec2 p3 = (p1 + 2.0) / size;
    vec2 p12 = (p1 + w2 / w12) / size;
    vec3 sum = texture(sampler2D(taa, texSampler), vec2(p12.x, p0.y)).rgb * (w12.x * w0.y)
        + texture(sampler2D(taa, texSampler), vec2(p0.x, p12.y)).rgb * (w0.x * w12.y)
        + texture(sampler2D(taa, texSampler), p12).rgb * (w12.x * w12.y)
        + texture(sampler2D(taa, texSampler), vec2(p3.x, p12.y)).rgb * (w3.x * w12.y)
        + texture(sampler2D(taa, texSampler), vec2(p12.x, p3.y)).rgb * (w12.x * w3.y);
    float weight = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return sum / weight;
}

// Accumulate the jittered frames: reproject last frame's result by depth and
// camera, clamp it to this frame's 3x3 neighborhood (Karis), and drop it
// where the depth disagrees.
void taaPass() {
    vec2 size = outputSize();
    vec2 render = frameSize();
    vec2 scale = render / size;
    ivec2 texel = ivec2(gl_FragCoord.xy);
    // This output pixel's center in render pixels, and the render pixel it
    // falls in. At render scale 1 they are the same pixel.
    vec2 renderPos = gl_FragCoord.xy * scale;
    ivec2 center = clamp(ivec2(floor(renderPos)), ivec2(0), ivec2(render) - 1);
    ivec2 limit = ivec2(render) - 1;
    // This frame's samples near the output pixel: each render pixel's sample
    // sits at its center plus JITTER, and counts by a Gaussian of its
    // distance to this output pixel's center, in output pixels. Gathering the
    // neighbors, not only the render pixel this one falls in, is what lets
    // the output pixels of one render pixel differ.
    vec2 jitter = temporal != 0u ? JITTER : vec2(0.0);
    vec3 gathered = vec3(0.0);
    float gatherWeight = 0.0;
    float lumas[9];
    vec3 lo = vec3(1e30);
    vec3 hi = vec3(-1e30);
    // Depth range of the neighborhood: history depth outside it was a
    // different surface.
    float tLo = MISS;
    float tHi = 0.0;
    bool anySky = false;
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            ivec2 at = clamp(center + ivec2(i, j), ivec2(0), limit);
            vec3 color = texelFetch(sampler2D(hdr, texSampler), at, 0).rgb;
            vec2 away = (vec2(at) + 0.5 + jitter - renderPos) / scale;
            float wk = exp(-dot(away, away) / (2.0 * 0.4 * 0.4));
            gathered += color * wk;
            gatherWeight += wk;
            lumas[(j + 1) * 3 + (i + 1)] = dot(color, vec3(0.2126, 0.7152, 0.0722));
            vec3 n = toYCoCg(color);
            lo = min(lo, n);
            hi = max(hi, n);
            float nt = texelFetch(sampler2D(gA, texSampler), at, 0).x;
            if (nt < MISS * 0.5) {
                tLo = min(tLo, nt);
                tHi = max(tHi, nt);
            } else {
                anySky = true;
            }
        }
    }
    // A silhouette: the jittered sample lands on sky one frame and on the
    // surface the next. Its history must survive that flip, or the edge
    // never accumulates coverage and stays a hard stair step. The color
    // clamp still keeps other surfaces from ghosting in.
    bool silhouette = anySky && tLo < MISS * 0.5;
    float t = texelFetch(sampler2D(gA, texSampler), center, 0).x;
    bool sky = t >= MISS * 0.5;

    // How much this frame saw of this output pixel: over the jitter cycle
    // every output pixel gets near samples.
    vec3 current = gathered / max(gatherWeight, 1e-6);
    float coverage = min(gatherWeight, 1.0);
    // A smooth upsample of this frame: the image where history is missing,
    // and a weak prior while the first samples gather.
    vec3 smoothed = hdrAtOutput(gl_FragCoord.xy / size);
    vec3 result = smoothed;
    float samples = coverage;

    Camera c = camera();
    float pixelAngle;
    vec3 dir = viewRay(c, gl_FragCoord.xy, size, pixelAngle);
    vec2 prevUv;
    float prevDist = TAA_SKY;
    bool onScreen = sky ? reprojectDirection(dir, c.fov, prevUv) : reproject(rayPoint(c, dir, t), c.fov, prevUv, prevDist);
    // How much last frame's result is worth: nothing without history, less
    // while the formula changes.
    float trust = 1.0 - newFrameWeight(0.0);
    if (onScreen && trust > 0.0) {
        // Depth from a bilinear tap: Catmull-Rom would ring across edges.
        vec4 history = texture(sampler2D(taa, texSampler), prevUv);
        history.rgb = max(catmullRomTaa(prevUv, size), vec3(0.0));
        bool historySky = history.a > 0.5 * TAA_SKY;
        // For a still camera prevDist is t; a moving one shifts the range by
        // how far the point moved relative to the camera.
        float shift = prevDist - t;
        bool depthMatch = silhouette || (sky
            ? historySky
            : (!historySky && history.a >= 0.95 * tLo + shift && history.a <= 1.05 * tHi + shift));
        if (depthMatch) {
            // Samples gathered so far. A still pixel keeps up to Still
            // Samples of them; one moving 2 output pixels a frame, 8.
            float motion = length(prevUv * size - gl_FragCoord.xy);
            float cap = mix(still_samples, 8.0, smoothstep(0.1, 2.0, motion));
            ivec2 prevTexel = clamp(ivec2(prevUv * size), ivec2(0), ivec2(size) - 1);
            float n = min(texelFetch(sampler2D(taaN, texSampler), prevTexel, 0).r * trust, cap);
            // The clamp keeps other surfaces from ghosting in, but it also
            // pulls accumulated sub-pixel detail back toward this frame's
            // coarse samples. Thin features keep their history (FSR 2's lock
            // test), and so do still pixels: they reproject exactly, and
            // trust already falls when the formula or the look changes. Only
            // where the history is this very surface: its depth matches this
            // pixel's own, which a filtered read across an edge does not.
            // Never the sky: flying forward it does not move on screen, and
            // a read beside geometry would carry the geometry's color along
            // as a trail.
            vec3 clamped = fromYCoCg(clamp(toYCoCg(history.rgb), lo, hi));
            float sameSurface = !sky && abs(history.a - (t + shift)) < 0.02 * t ? trust : 0.0;
            float still = (1.0 - smoothstep(0.05, 0.5, motion)) * sameSurface;
            float slow = (1.0 - smoothstep(0.5, 2.0, motion)) * sameSurface;
            clamped = mix(clamped, history.rgb, thinFeature(lumas) ? slow : still);
            // A running average, each term weighted by inverse luminance
            // (Karis) so a pixel bright in only some frames, a rim sparkle,
            // cannot dominate it.
            vec3 luma = vec3(0.2126, 0.7152, 0.0722);
            float wc = coverage / (1.0 + dot(current, luma));
            float wh = n / (1.0 + dot(clamped, luma));
            float ws = 0.25 * max(1.0 - n, 0.0) / (1.0 + dot(smoothed, luma));
            result = (current * wc + clamped * wh + smoothed * ws) / max(wc + wh + ws, 1e-6);
            samples = min(n + coverage, cap);
        }
    }
    out0 = vec4(result, sky ? TAA_SKY : t);
    out1 = vec4(samples, 0.0, 0.0, 0.0);
}

// ── Output pass ──────────────────────────────────────────────────────────

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Depth of field, bloom, lens effects and grade. Linear and unclamped:
// Varda tonemaps downstream.
vec3 composite(ivec2 texel) {
    vec2 size = outputSize();
    vec2 uv = (vec2(texel) + 0.5) / size;
    vec3 color = resolved(texel);
    if (aberration > 0.0) {
        vec2 shift = (uv - 0.5) * aberration * 0.01;
        ivec2 limit = ivec2(size) - 1;
        color.r = resolved(clamp(ivec2((uv - shift) * size), ivec2(0), limit)).r;
        color.b = resolved(clamp(ivec2((uv + shift) * size), ivec2(0), limit)).b;
    }
    if (dof_mode != 0) {
        vec4 blurred = texture(sampler2D(dof, texSampler), uv);
        color = mix(color, blurred.rgb, smoothstep(0.5, 2.0, blurred.a));
    }
    if (bloom > 0.0) {
        vec3 glowSum = texture(sampler2D(bloom1, texSampler), uv).rgb
            + texture(sampler2D(bloom2, texSampler), uv).rgb
            + texture(sampler2D(bloom3, texSampler), uv).rgb;
        color += glowSum * bloom;
    }
    color *= exp2(exposure);
    // Contrast about middle grey, so it does not darken everything.
    const float PIVOT = 0.18;
    color = PIVOT * pow(max(color, vec3(0.0)) / PIVOT, vec3(contrast));
    float luma = dot(color, vec3(0.2126, 0.7152, 0.0722));
    color = mix(vec3(luma), color, saturation);
    vec2 centered = uv - 0.5;
    color *= 1.0 - vignette * dot(centered, centered) * 2.0;
    color += (hash12(gl_FragCoord.xy + float(FRAMEINDEX) * 17.0) - 0.5) * grain * max(luma, 0.05);
    return color;
}

vec3 alarm(vec2 frag) {
    vec2 cell = floor(frag / 16.0);
    return mod(cell.x + cell.y, 2.0) < 1.0 ? vec3(1.0, 0.0, 1.0) : vec3(0.0);
}

void outputPass() {
    ivec2 texel = ivec2(gl_FragCoord.xy);
    vec4 a = texelFetch(sampler2D(gA, texSampler), toRender(texel), 0);
    vec4 b = texelFetch(sampler2D(gB, texSampler), toRender(texel), 0);
    if (debug_view == 6) {
        out0 = a;
        return;
    }
    if (debug_view == 1 || debug_view == 4) {
        out0 = vec4(b.rgb, 1.0);
        return;
    }
    bool surface = a.x < MISS * 0.5;
    if (debug_view == 2) {
        out0 = vec4(surface ? b.xyz * 0.5 + 0.5 : vec3(0.0), 1.0);
        return;
    }
    if (debug_view == 3) {
        out0 = vec4(vec3(surface ? 1.0 / (1.0 + a.x) : 0.0), 1.0);
        return;
    }
    if (debug_view == 5) {
        out0 = vec4(vec3(a.y / max(max_iterations, 1.0)), 1.0);
        return;
    }
    if (debug_view == 7) {
        vec2 sh = upsampleShade(gl_FragCoord.xy * frameSize() / outputSize(), a.x);
        out0 = vec4(sh.x, sh.y, 0.0, 1.0);
        return;
    }
    if (debug_view == 8) {
        // White where the shade pass computed this frame.
        float computed = texelFetch(sampler2D(shade, texSampler), toRender(texel), 0).a;
        out0 = vec4(vec3(computed), 1.0);
        return;
    }
    out0 = vec4(composite(texel), 1.0);
}

void main() {
    applyLook();
    if (!flightValid()) {
        vec3 c = alarm(gl_FragCoord.xy);
        out0 = vec4(c, 1.0);
        out1 = vec4(0.0);
        return;
    }
    out1 = vec4(0.0);
    out2 = vec4(0.0);
    // Render-size passes draw only the live part of their targets.
    float divisor = PASS == PASS_TILES ? float(TILE)
        : PASS == PASS_SHADE ? 1.0
        : PASS == PASS_VOLUME ? 4.0
        : (PASS == PASS_GBUFFER || PASS == PASS_LIGHT) ? 1.0 : 0.0;
    if (divisor > 0.0 && outsideLive(divisor)) {
        out0 = vec4(0.0);
        return;
    }
    if (PASS == PASS_GBUFFER) {
        gbufferPass();
        return;
    }
    if (PASS == PASS_TILES) {
        tilesPass();
    } else if (PASS == PASS_VOLUME) {
        volumePass();
    } else if (PASS == PASS_TAA) {
        taaPass();
    } else if (PASS == PASS_SHARP) {
        sharpPass();
    } else if (PASS == PASS_DOF) {
        dofPass();
    } else if (PASS == PASS_BLOOM1) {
        bloomPass(1);
    } else if (PASS == PASS_BLOOM2) {
        bloomPass(2);
    } else if (PASS == PASS_BLOOM3) {
        bloomPass(3);
    } else if (PASS == PASS_SHADE) {
        shadePass();
    } else if (PASS == PASS_LIGHT) {
        lightPass();
    } else {
        outputPass();
    }
}
