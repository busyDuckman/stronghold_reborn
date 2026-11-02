#version 330
#ifdef GL_ES
    precision highp float;
    precision highp int;
#endif

// Engine-provided vertex inputs. Every shader in a project shares this file's shape, so a
// .vert is usually a copy of this one.
layout(location = 0) in vec3 position;
layout(location = 1) in vec2 texcoord;

uniform mat4 Model;
uniform mat4 Projection;

out vec2 v_uv;

void main() {
    v_uv = texcoord;
    gl_Position = Projection * Model * vec4(position, 1.0);
}
