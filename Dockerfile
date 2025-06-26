FROM --platform=linux/amd64 rust:1.87.0

ENV DFX_VERSION=0.27.0
ENV CANDID_EXTRACTOR_VERSION=0.1.6

# Install dfx
RUN DFXVM_INIT_YES=true sh -ci "$(curl -fsSL https://internetcomputer.org/install.sh)"
ENV PATH="/root/.local/share/dfx/bin:$PATH"

# Install candid-extractor
RUN curl -sL https://github.com/dfinity/candid-extractor/releases/download/${CANDID_EXTRACTOR_VERSION}/candid-extractor-x86_64-unknown-linux-gnu.tar.gz -o candid-extractor.tar.gz
RUN tar -xzf candid-extractor.tar.gz
RUN rm candid-extractor.tar.gz
RUN chmod +x candid-extractor
RUN mv candid-extractor /usr/local/bin/candid-extractor

WORKDIR /app
