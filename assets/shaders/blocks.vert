#version 330 core
layout(location=0) in vec3 position;
layout(location=1) in vec2 uv;
layout(location=2) in vec3 normal;
uniform mat4 mvp;
out vec2 tex;
out float shade;
void main(){
    tex=uv;
    // Diagnostic directional shading; vanilla light propagation/AO is a separate layer.
    shade=length(normal)<0.1?1.0:0.65+0.35*max(dot(normalize(normal),normalize(vec3(0.4,1.0,0.25))),0.0);
    gl_Position=mvp*vec4(position,1.0);
}
