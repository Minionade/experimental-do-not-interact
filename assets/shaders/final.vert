#version 330 core
in vec3 a_position;
in vec2 a_uv;
out vec2 texcoord;
void main() { texcoord = a_uv; gl_Position = vec4(a_position, 1.0); }
