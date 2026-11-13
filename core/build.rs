fn main() {
    println!("cargo:rerun-if-changed=../assets/qualifications/meeting-support-cloud-v1.json");
    println!("cargo:rerun-if-changed=../assets/qualifications/meeting-assistance.json");
}
