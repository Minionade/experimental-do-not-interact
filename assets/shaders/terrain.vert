#version 330 core
in vec3 a_position;
in vec3 a_normal;
in vec4 a_color;
uniform mat4 u_model_view;
uniform mat4 u_projection;
out vec3 color;
out vec3 normal;
void main() {
    gl_Position = u_projection * u_model_view * vec4(a_position, 1.0);
    color = a_color.rgb;
    normal = normalize(mat3(u_model_view) * a_normal);
}
